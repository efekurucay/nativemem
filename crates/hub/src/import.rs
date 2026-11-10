//! Reads the MCP server entries of the four agents (read-only) and turns them into hub entries.
//! Formats: `wiki/agents/*.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::{validate_name, Auth, HubConfig, ServerSpec, Transport, HUB_NAME};
use crate::error::Result;
use crate::secrets::{validate_secret_id, SecretStore, SECRET_PREFIX};

/// The agent homes the CLI resolved.
pub struct AgentRoots<'a> {
    pub claude: &'a Path,
    pub codex: &'a Path,
    pub opencode: &'a Path,
    pub pi: &'a Path,
}

/// Claude keeps its global state in `.claude.json`: inside the config directory when that is a
/// custom one, next to `~/.claude` otherwise.
pub fn claude_config_path(root: &Path) -> PathBuf {
    if root.file_name().is_some_and(|n| n == ".claude") {
        root.with_file_name(".claude.json")
    } else {
        root.join(".claude.json")
    }
}

/// An entry as one agent holds it, normalised. Values are still literal.
struct Found {
    agent: &'static str,
    name: String,
    spec: ServerSpec,
}

pub struct Skipped {
    pub agent: String,
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not in the hub yet.
    Add,
    /// The hub already holds this exact entry.
    Present,
    /// The name exists with a different config, in another agent or in the hub.
    Conflict,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Add => "add",
            Status::Present => "present",
            Status::Conflict => "conflict",
        }
    }
}

/// A secret the import creates. The value is never printed: this type has no `Debug`.
pub struct PendingSecret {
    pub id: String,
    /// `None` when the agent gave a reference to an environment variable or a command, which the
    /// hub cannot evaluate: the id is created unset and the user sets it.
    value: Option<String>,
}

pub struct PlannedServer {
    pub name: String,
    /// The hub entry, with `secret:<id>` references.
    pub spec: ServerSpec,
    /// Agents that hold this entry, plus `hub` when the hub holds the name already.
    pub agents: Vec<String>,
    pub status: Status,
    pub secrets: Vec<PendingSecret>,
}

#[derive(Default)]
pub struct ImportPlan {
    pub servers: Vec<PlannedServer>,
    pub skipped: Vec<Skipped>,
    pub warnings: Vec<String>,
}

// ---- reading the agents ----

fn str_map(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(map)) = value {
        for (k, v) in map {
            match v {
                Value::String(s) => out.insert(k.clone(), s.clone()),
                Value::Number(_) | Value::Bool(_) => out.insert(k.clone(), v.to_string()),
                _ => None,
            };
        }
    }
    out
}

fn str_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn oauth(value: Option<&Value>) -> Auth {
    match value {
        Some(Value::Bool(false)) | Some(Value::Null) | None => Auth::None,
        Some(_) => Auth::Oauth,
    }
}

fn opt_str(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

struct Reader {
    agent: &'static str,
    found: Vec<Found>,
    skipped: Vec<Skipped>,
    warnings: Vec<String>,
}

impl Reader {
    fn new(agent: &'static str) -> Self {
        Reader {
            agent,
            found: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn skip(&mut self, name: &str, reason: &str) {
        self.skipped.push(Skipped {
            agent: self.agent.to_string(),
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }

    fn ignored(&mut self, name: &str, obj: &Map<String, Value>, known: &[&str]) {
        let mut extra: Vec<&str> = obj
            .keys()
            .map(String::as_str)
            .filter(|k| !known.contains(k))
            .collect();
        if !extra.is_empty() {
            extra.sort();
            self.warnings.push(format!(
                "{} {name}: keys not carried over: {}",
                self.agent,
                extra.join(", ")
            ));
        }
    }

    fn add(&mut self, name: &str, spec: ServerSpec) {
        match self
            .found
            .iter_mut()
            .find(|f| f.name == name && f.agent == self.agent)
        {
            Some(existing) => existing.spec = spec,
            None => self.found.push(Found {
                agent: self.agent,
                name: name.to_string(),
                spec,
            }),
        }
    }

    fn warn_unreadable(&mut self, path: &Path, what: &str) {
        self.warnings.push(format!(
            "{}: cannot read {what} {}",
            self.agent,
            path.display()
        ));
    }

    /// Claude Code and Pi share one entry shape.
    fn json_servers(&mut self, servers: &Map<String, Value>) {
        for (name, entry) in servers {
            let Some(obj) = entry.as_object() else {
                self.skip(name, "entry is not an object");
                continue;
            };
            let declared = obj.get("type").and_then(Value::as_str);
            let transport = match (
                declared,
                obj.contains_key("command"),
                obj.contains_key("url"),
            ) {
                (Some("sse"), _, _) => {
                    self.skip(name, "sse transport is not supported by the hub");
                    continue;
                }
                (Some("stdio"), _, _) | (None, true, _) => Transport::Stdio,
                (Some("http"), _, _) | (None, false, true) => Transport::Http,
                _ => {
                    self.skip(name, "neither a command nor a url");
                    continue;
                }
            };
            self.ignored(
                name,
                obj,
                &[
                    "type", "command", "args", "env", "cwd", "url", "headers", "oauth", "enabled",
                ],
            );
            let mut spec = match transport {
                Transport::Stdio => match opt_str(obj.get("command")) {
                    Some(c) => ServerSpec::stdio(c),
                    None => {
                        self.skip(name, "stdio entry without a command");
                        continue;
                    }
                },
                Transport::Http => match opt_str(obj.get("url")) {
                    Some(u) => ServerSpec::http(u),
                    None => {
                        self.skip(name, "http entry without a url");
                        continue;
                    }
                },
            };
            match transport {
                Transport::Stdio => {
                    spec.args = str_list(obj.get("args"));
                    spec.env = str_map(obj.get("env"));
                    spec.cwd = opt_str(obj.get("cwd"));
                }
                Transport::Http => {
                    spec.headers = str_map(obj.get("headers"));
                    spec.auth = oauth(obj.get("oauth"));
                }
            }
            spec.enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            self.add(name, spec);
        }
    }

    fn opencode_servers(&mut self, servers: &Map<String, Value>) {
        for (name, entry) in servers {
            let Some(obj) = entry.as_object() else {
                self.skip(name, "entry is not an object");
                continue;
            };
            self.ignored(
                name,
                obj,
                &[
                    "type",
                    "command",
                    "environment",
                    "cwd",
                    "url",
                    "headers",
                    "oauth",
                    "enabled",
                ],
            );
            let mut spec = match obj.get("type").and_then(Value::as_str) {
                Some("local") => {
                    let mut command = str_list(obj.get("command"));
                    if command.is_empty() {
                        self.skip(name, "local entry without a command");
                        continue;
                    }
                    let mut spec = ServerSpec::stdio(command.remove(0));
                    spec.args = command;
                    spec.env = str_map(obj.get("environment"));
                    spec.cwd = opt_str(obj.get("cwd"));
                    spec
                }
                Some("remote") => {
                    let Some(url) = opt_str(obj.get("url")) else {
                        self.skip(name, "remote entry without a url");
                        continue;
                    };
                    let mut spec = ServerSpec::http(url);
                    spec.headers = str_map(obj.get("headers"));
                    spec.auth = oauth(obj.get("oauth"));
                    spec
                }
                _ => {
                    self.skip(name, "type is neither local nor remote");
                    continue;
                }
            };
            spec.enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            self.add(name, spec);
        }
    }

    fn codex_servers(&mut self, servers: &toml::Table) {
        for (name, entry) in servers {
            let Some(table) = entry.as_table() else {
                self.skip(name, "entry is not a table");
                continue;
            };
            // Through JSON the table reads like the other agents' entries.
            let Ok(Value::Object(obj)) = serde_json::to_value(table) else {
                self.skip(name, "entry cannot be read");
                continue;
            };
            self.ignored(
                name,
                &obj,
                &[
                    "command",
                    "args",
                    "env",
                    "cwd",
                    "url",
                    "http_headers",
                    "enabled",
                ],
            );
            let mut spec = if let Some(command) = opt_str(obj.get("command")) {
                let mut spec = ServerSpec::stdio(command);
                spec.args = str_list(obj.get("args"));
                spec.env = str_map(obj.get("env"));
                spec.cwd = opt_str(obj.get("cwd"));
                spec
            } else if let Some(url) = opt_str(obj.get("url")) {
                let mut spec = ServerSpec::http(url);
                spec.headers = str_map(obj.get("http_headers"));
                spec
            } else {
                self.skip(name, "neither a command nor a url");
                continue;
            };
            spec.enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            self.add(name, spec);
        }
    }
}

/// JSON with comments and trailing commas, as OpenCode accepts.
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    // Trailing commas, now that comments are gone.
    let chars: Vec<char> = out.chars().collect();
    let mut cleaned = String::with_capacity(out.len());
    let mut in_string = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            cleaned.push(c);
            if c == '"' && chars[..i].iter().rev().take_while(|&&p| p == '\\').count() % 2 == 0 {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        cleaned.push(c);
    }
    cleaned
}

fn read_json(reader: &mut Reader, path: &Path, jsonc: bool) -> Option<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            reader.warn_unreadable(path, "the file");
            return None;
        }
    };
    let text = if jsonc { strip_jsonc(&text) } else { text };
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(_) => {
            reader.warn_unreadable(path, "invalid JSON in");
            None
        }
    }
}

fn read_agents(roots: &AgentRoots) -> Vec<Reader> {
    let mut claude = Reader::new("claude");
    if let Some(doc) = read_json(&mut claude, &claude_config_path(roots.claude), false) {
        if let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) {
            claude.json_servers(servers);
        }
    }

    let mut codex = Reader::new("codex");
    let path = roots.codex.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => match text.parse::<toml::Table>() {
            Ok(doc) => {
                if let Some(servers) = doc.get("mcp_servers").and_then(|v| v.as_table()) {
                    codex.codex_servers(servers);
                }
            }
            Err(_) => codex.warn_unreadable(&path, "invalid TOML in"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => codex.warn_unreadable(&path, "the file"),
    }

    let mut opencode = Reader::new("opencode");
    for file in ["opencode.json", "opencode.jsonc"] {
        if let Some(doc) = read_json(&mut opencode, &roots.opencode.join(file), true) {
            if let Some(servers) = doc.get("mcp").and_then(Value::as_object) {
                opencode.opencode_servers(servers);
            }
        }
    }

    let mut pi = Reader::new("pi");
    if let Some(doc) = read_json(&mut pi, &roots.pi.join("mcp.json"), false) {
        if let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) {
            pi.json_servers(servers);
        }
    }
    vec![claude, codex, opencode, pi]
}

// ---- planning ----

/// A value an agent resolves itself: `${VAR}` or `!command`.
fn is_indirect(value: &str) -> bool {
    (value.starts_with("${") && value.ends_with('}')) || value.starts_with('!')
}

fn secret_id(server: &str, key: &str) -> String {
    let clean: String = format!("{server}-{key}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    clean
}

/// Moves the literal env and header values of `spec` out of it: each becomes
/// `secret:<server>-<key>`.
fn extract(
    server: &str,
    spec: &ServerSpec,
    warnings: &mut Vec<String>,
) -> (ServerSpec, Vec<PendingSecret>) {
    let mut out = spec.clone();
    let mut secrets = Vec::new();
    let mut move_values = |values: &mut BTreeMap<String, String>| {
        for (key, value) in values.iter_mut() {
            if value.starts_with(SECRET_PREFIX) {
                continue;
            }
            let id = secret_id(server, key);
            let stored = if is_indirect(value) {
                warnings.push(format!(
                    "{server}: {key} refers to an environment variable or a command; set secret {id} with `nativemem mcp set-secret`"
                ));
                None
            } else {
                Some(value.clone())
            };
            *value = format!("{SECRET_PREFIX}{id}");
            secrets.push(PendingSecret { id, value: stored });
        }
    };
    move_values(&mut out.env);
    move_values(&mut out.headers);
    (out, secrets)
}

pub fn plan(roots: &AgentRoots, existing: &HubConfig) -> ImportPlan {
    let mut plan = ImportPlan::default();
    let mut groups: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for reader in read_agents(roots) {
        plan.skipped.extend(reader.skipped);
        plan.warnings.extend(reader.warnings);
        for f in reader.found {
            if f.name == HUB_NAME {
                continue;
            }
            if let Err(e) = validate_name(&f.name).and_then(|()| f.spec.validate()) {
                plan.skipped.push(Skipped {
                    agent: f.agent.to_string(),
                    name: f.name,
                    reason: e.to_string(),
                });
                continue;
            }
            groups.entry(f.name.clone()).or_default().push(f);
        }
    }
    for (name, group) in groups {
        let first = &group[0];
        let mut agents: Vec<String> = Vec::new();
        let mut conflict = false;
        for f in &group {
            if !agents.contains(&f.agent.to_string()) {
                agents.push(f.agent.to_string());
            }
            conflict |= f.spec != first.spec;
        }
        let mut warnings = Vec::new();
        let (spec, secrets) = extract(&name, &first.spec, &mut warnings);
        let status = if conflict {
            Status::Conflict
        } else {
            match existing.servers.get(&name) {
                None => Status::Add,
                Some(held) if *held == spec => Status::Present,
                Some(_) => {
                    agents.push("hub".to_string());
                    Status::Conflict
                }
            }
        };
        if status == Status::Add {
            plan.warnings.extend(warnings);
        }
        plan.servers.push(PlannedServer {
            name,
            spec,
            agents,
            status,
            secrets,
        });
    }
    plan
}

/// Stores the secrets of every entry to add, then adds the entries to `config`. Returns how many
/// entries were added. The caller saves `config`.
pub fn apply(plan: &ImportPlan, config: &mut HubConfig, store: &dyn SecretStore) -> Result<usize> {
    let mut added = 0;
    for server in plan.servers.iter().filter(|s| s.status == Status::Add) {
        for secret in &server.secrets {
            validate_secret_id(&secret.id)?;
            if let Some(value) = &secret.value {
                store.set(&secret.id, value)?;
            }
        }
        config
            .servers
            .insert(server.name.clone(), server.spec.clone());
        added += 1;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_comments_and_trailing_commas_are_removed() {
        let text = r#"{
            // line
            "a": "x // not a comment", /* block */
            "b": ["1", "2",],
            "c": {"d": "e\"//",},
        }"#;
        let v: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(v["a"], "x // not a comment");
        assert_eq!(v["b"], serde_json::json!(["1", "2"]));
        assert_eq!(v["c"]["d"], "e\"//");
    }

    #[test]
    fn claude_config_sits_next_to_the_default_home_and_inside_a_custom_one() {
        assert_eq!(
            claude_config_path(Path::new("/Users/x/.claude")),
            PathBuf::from("/Users/x/.claude.json")
        );
        assert_eq!(
            claude_config_path(Path::new("/tmp/cfg")),
            PathBuf::from("/tmp/cfg/.claude.json")
        );
    }

    #[test]
    fn secret_ids_are_clean() {
        assert_eq!(secret_id("gh", "API KEY"), "gh-API_KEY");
        assert!(validate_secret_id(&secret_id("a", &"k".repeat(300))).is_ok());
    }
}
