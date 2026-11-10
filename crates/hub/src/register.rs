//! Registers the hub in each agent's own MCP configuration: one entry named `nativemem` that runs
//! `<this binary> mcp serve`. Only that entry is written; everything else in the file stays as it
//! was. Formats: `wiki/agents/*.md`, decision: `wiki/decisions/mcp-hub.md`.

use std::path::{Path, PathBuf};

use jsonc_parser::cst::{CstInputValue, CstNode, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use nativemem_core::fsx::{atomic_write, backup, BackupSession};
use nativemem_core::model::AgentId;
use serde::Serialize;
use serde_json::{json, Map, Value};
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::config::HUB_NAME;
use crate::error::{HubError, Result};
use crate::import::{claude_config_path, AgentRoots};

/// The agents that take a registration, in the order they are processed.
pub const AGENTS: [&str; 4] = ["claude", "codex", "opencode", "pi"];

/// How many times a write starts over when the file changed under it.
const ATTEMPTS: usize = 3;

/// Codex cuts a tool call after `tool_timeout_sec`; subagent runs behind the hub take long.
const CODEX_TOOL_TIMEOUT_SEC: i64 = 1800;
/// Codex gives up on a server that does not start within `startup_timeout_sec`; the hub connects
/// its upstreams (up to 20 s each, concurrently) before it answers.
const CODEX_STARTUP_TIMEOUT_SEC: i64 = 60;

/// What gets registered: the command and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
}

impl Entry {
    fn argv(&self) -> Vec<&str> {
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect()
    }
}

/// The result for one agent. `status` is one of `installed`, `unchanged`, `removed`, `absent`,
/// `registered`, `outdated`, `missing`, `not_installed` or `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentOutcome {
    pub agent: &'static str,
    pub status: &'static str,
    pub path: Option<PathBuf>,
    pub warnings: Vec<String>,
    /// Code and message of the failure when `status` is `error`.
    pub error: Option<(&'static str, String)>,
}

impl AgentOutcome {
    fn new(agent: &'static str, status: &'static str, path: Option<PathBuf>) -> Self {
        AgentOutcome {
            agent,
            status,
            path,
            warnings: Vec::new(),
            error: None,
        }
    }

    fn failed(agent: &'static str, path: Option<PathBuf>, e: HubError) -> Self {
        AgentOutcome {
            error: Some((e.code(), e.to_string())),
            ..AgentOutcome::new(agent, "error", path)
        }
    }
}

/// Where the backups of a run go.
pub struct Backups<'a> {
    pub state: &'a Path,
    pub session: &'a BackupSession,
}

/// How an agent's file holds the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Missing,
    Current,
    Outdated,
}

trait Format {
    /// The text with the entry in place; `None` when it already is.
    fn install(&self, text: Option<&str>, entry: &Entry) -> Result<Option<String>>;
    /// The text without the entry; `None` when there was none.
    fn uninstall(&self, text: &str) -> Result<Option<String>>;
    fn state(&self, text: &str, entry: &Entry) -> Result<State>;
}

fn parse_error(what: &str) -> HubError {
    HubError::Parse(what.to_string())
}

/// A parse failure names the file it came from.
fn located(path: &Path, e: HubError) -> HubError {
    match e {
        HubError::Parse(m) => HubError::Parse(format!("cannot parse {}: {m}", path.display())),
        other => other,
    }
}

fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().map(|v| json!(v)).collect())
}

// ---- plain JSON: Claude Code and Pi -------------------------------------------------------

/// A JSON file with the entry at `mcpServers.nativemem`, read and written with `serde_json` in
/// file order.
struct JsonServers {
    /// Claude's entries carry `type` and `env`; Pi's take `command` and `args` only.
    claude: bool,
}

impl JsonServers {
    fn wanted(&self, entry: &Entry) -> Vec<(&'static str, Value)> {
        let args = strings(&entry.args.iter().map(String::as_str).collect::<Vec<_>>());
        if self.claude {
            vec![
                ("type", json!("stdio")),
                ("command", json!(entry.command)),
                ("args", args),
                ("env", json!({})),
            ]
        } else {
            vec![("command", json!(entry.command)), ("args", args)]
        }
    }

    fn parse(text: Option<&str>) -> Result<Value> {
        match text.map(str::trim) {
            None | Some("") => Ok(json!({})),
            Some(t) => serde_json::from_str(t).map_err(|e| parse_error(&e.to_string())),
        }
    }

    fn servers(doc: &mut Value) -> Result<&mut Map<String, Value>> {
        let root = doc
            .as_object_mut()
            .ok_or_else(|| parse_error("the top level is not an object"))?;
        root.entry("mcpServers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| parse_error("mcpServers is not an object"))
    }
}

/// The indentation of the first indented line, two spaces without one.
fn indent_of(text: &str) -> String {
    text.lines()
        .skip(1)
        .find_map(|line| {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            (!indent.is_empty() && indent.len() < line.len()).then_some(indent)
        })
        .unwrap_or_else(|| "  ".to_string())
}

fn render_json(doc: &Value, original: Option<&str>) -> Result<String> {
    let indent = original.map(indent_of).unwrap_or_else(|| "  ".to_string());
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    doc.serialize(&mut serializer)
        .map_err(|e| parse_error(&e.to_string()))?;
    let mut text = String::from_utf8(out).map_err(|e| parse_error(&e.to_string()))?;
    if original.is_none_or(|o| o.ends_with('\n')) {
        text.push('\n');
    }
    Ok(text)
}

impl Format for JsonServers {
    fn install(&self, text: Option<&str>, entry: &Entry) -> Result<Option<String>> {
        let mut doc = Self::parse(text)?;
        let servers = Self::servers(&mut doc)?;
        let mut changed = false;
        match servers.get_mut(HUB_NAME).and_then(Value::as_object_mut) {
            Some(existing) => {
                for (key, wanted) in self.wanted(entry) {
                    if existing.get(key) != Some(&wanted) {
                        existing.insert(key.to_string(), wanted);
                        changed = true;
                    }
                }
            }
            None => {
                servers.insert(
                    HUB_NAME.to_string(),
                    Value::Object(
                        self.wanted(entry)
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect(),
                    ),
                );
                changed = true;
            }
        }
        if !changed {
            return Ok(None);
        }
        render_json(&doc, text).map(Some)
    }

    fn uninstall(&self, text: &str) -> Result<Option<String>> {
        let mut doc = Self::parse(Some(text))?;
        let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) else {
            return Ok(None);
        };
        if servers.shift_remove(HUB_NAME).is_none() {
            return Ok(None);
        }
        render_json(&doc, Some(text)).map(Some)
    }

    fn state(&self, text: &str, entry: &Entry) -> Result<State> {
        let doc = Self::parse(Some(text))?;
        let Some(existing) = doc.pointer(&format!("/mcpServers/{HUB_NAME}")) else {
            return Ok(State::Missing);
        };
        let current = self
            .wanted(entry)
            .iter()
            .all(|(key, wanted)| existing.get(key) == Some(wanted));
        Ok(if current {
            State::Current
        } else {
            State::Outdated
        })
    }
}

// ---- TOML: Codex ---------------------------------------------------------------------------

/// `[mcp_servers.nativemem]` in `config.toml`, edited with `toml_edit` so every other byte stays.
struct CodexToml;

impl CodexToml {
    fn parse(text: &str) -> Result<DocumentMut> {
        text.parse::<DocumentMut>()
            .map_err(|e| parse_error(&e.to_string()))
    }

    fn args_value(entry: &Entry) -> toml_edit::Value {
        Array::from_iter(entry.args.iter().map(String::as_str)).into()
    }

    fn args_match(item: &Item, entry: &Entry) -> bool {
        item.as_array().is_some_and(|a| {
            a.len() == entry.args.len()
                && a.iter()
                    .zip(&entry.args)
                    .all(|(have, want)| have.as_str() == Some(want.as_str()))
        })
    }
}

impl Format for CodexToml {
    fn install(&self, text: Option<&str>, entry: &Entry) -> Result<Option<String>> {
        let original = text.unwrap_or_default();
        let mut doc = Self::parse(original)?;
        let servers = match doc.get_mut("mcp_servers") {
            Some(item) => item,
            None => {
                let mut table = Table::new();
                table.set_implicit(true);
                doc.insert("mcp_servers", Item::Table(table));
                &mut doc["mcp_servers"]
            }
        }
        .as_table_like_mut()
        .ok_or_else(|| parse_error("mcp_servers is not a table"))?;
        match servers.get_mut(HUB_NAME) {
            Some(item) => {
                let table = item
                    .as_table_like_mut()
                    .ok_or_else(|| parse_error("mcp_servers.nativemem is not a table"))?;
                if table.get("command").and_then(Item::as_str) != Some(entry.command.as_str()) {
                    table.insert("command", value(entry.command.as_str()));
                }
                if !table
                    .get("args")
                    .is_some_and(|item| Self::args_match(item, entry))
                {
                    table.insert("args", Item::Value(Self::args_value(entry)));
                }
                if !table.contains_key("tool_timeout_sec") {
                    table.insert("tool_timeout_sec", value(CODEX_TOOL_TIMEOUT_SEC));
                }
                if !table.contains_key("startup_timeout_sec") {
                    table.insert("startup_timeout_sec", value(CODEX_STARTUP_TIMEOUT_SEC));
                }
            }
            None => {
                let mut table = Table::new();
                table.insert("command", value(entry.command.as_str()));
                table.insert("args", Item::Value(Self::args_value(entry)));
                table.insert("tool_timeout_sec", value(CODEX_TOOL_TIMEOUT_SEC));
                table.insert("startup_timeout_sec", value(CODEX_STARTUP_TIMEOUT_SEC));
                servers.insert(HUB_NAME, Item::Table(table));
            }
        }
        let updated = doc.to_string();
        Ok((updated != original).then_some(updated))
    }

    fn uninstall(&self, text: &str) -> Result<Option<String>> {
        let mut doc = Self::parse(text)?;
        let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) else {
            return Ok(None);
        };
        if servers.remove(HUB_NAME).is_none() {
            return Ok(None);
        }
        Ok(Some(doc.to_string()))
    }

    fn state(&self, text: &str, entry: &Entry) -> Result<State> {
        let doc = Self::parse(text)?;
        let Some(table) = doc
            .get("mcp_servers")
            .and_then(Item::as_table_like)
            .and_then(|t| t.get(HUB_NAME))
            .and_then(Item::as_table_like)
        else {
            return Ok(State::Missing);
        };
        let current = table.get("command").and_then(Item::as_str) == Some(entry.command.as_str())
            && table
                .get("args")
                .is_some_and(|item| Self::args_match(item, entry));
        Ok(if current {
            State::Current
        } else {
            State::Outdated
        })
    }
}

// ---- JSONC: OpenCode -----------------------------------------------------------------------

/// `mcp.nativemem` in `opencode.json` or `opencode.jsonc`, or `mcp.servers.nativemem` when the
/// file keeps its servers there. Edited as a concrete syntax tree: comments, formatting and every
/// other property stay.
struct OpenCodeJsonc;

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::Object(map) => {
            CstInputValue::Object(map.iter().map(|(k, v)| (k.clone(), input(v))).collect())
        }
    }
}

/// An entry of `mcp.servers` is an object that is not itself an MCP server definition: a server
/// named `servers` would carry `type`, `command` or `url`.
fn is_server_list(object: &CstObject) -> bool {
    !["type", "command", "url"]
        .iter()
        .any(|key| object.get(key).is_some())
}

impl OpenCodeJsonc {
    fn wanted(entry: &Entry) -> Vec<(&'static str, Value)> {
        vec![
            ("type", json!("local")),
            ("command", strings(&entry.argv())),
            ("enabled", json!(true)),
        ]
    }

    fn parse(text: &str) -> Result<CstRootNode> {
        CstRootNode::parse(text, &ParseOptions::default()).map_err(|e| parse_error(&e.to_string()))
    }

    /// The object the entry lives in: `mcp.servers` when that is the file's layout, else `mcp`.
    fn container(root: &CstRootNode, create: bool) -> Result<Option<CstObject>> {
        let top = if create {
            root.object_value_or_create()
        } else {
            root.object_value()
        };
        let Some(top) = top else {
            return if create {
                Err(parse_error("the top level is not an object"))
            } else {
                Ok(None)
            };
        };
        let mcp = if create {
            Some(
                top.object_value_or_create("mcp")
                    .ok_or_else(|| parse_error("mcp is not an object"))?,
            )
        } else {
            top.object_value("mcp")
        };
        Ok(mcp.map(|mcp| match mcp.object_value("servers") {
            Some(servers) if is_server_list(&servers) => servers,
            _ => mcp,
        }))
    }

    fn entry(container: &CstObject) -> Option<CstNode> {
        container.get(HUB_NAME)?.value()
    }
}

impl Format for OpenCodeJsonc {
    fn install(&self, text: Option<&str>, entry: &Entry) -> Result<Option<String>> {
        let original = match text.map(str::trim) {
            None | Some("") => "{}\n",
            Some(_) => text.unwrap_or_default(),
        };
        let root = Self::parse(original)?;
        let container = Self::container(&root, true)?
            .ok_or_else(|| parse_error("the top level is not an object"))?;
        let wanted = Self::wanted(entry);
        match container.get(HUB_NAME) {
            None => {
                container.append(
                    HUB_NAME,
                    CstInputValue::Object(
                        wanted
                            .iter()
                            .map(|(k, v)| (k.to_string(), input(v)))
                            .collect(),
                    ),
                );
            }
            Some(prop) => match prop.object_value() {
                Some(existing) => {
                    for (key, want) in &wanted {
                        match existing.get(key) {
                            Some(p) => {
                                if p.value().and_then(|v| v.to_serde_value()).as_ref() != Some(want)
                                {
                                    p.set_value(input(want));
                                }
                            }
                            None => {
                                existing.append(key, input(want));
                            }
                        }
                    }
                }
                None => prop.set_value(CstInputValue::Object(
                    wanted
                        .iter()
                        .map(|(k, v)| (k.to_string(), input(v)))
                        .collect(),
                )),
            },
        }
        let updated = root.to_string();
        Ok((text != Some(updated.as_str())).then_some(updated))
    }

    fn uninstall(&self, text: &str) -> Result<Option<String>> {
        let root = Self::parse(text)?;
        let Some(container) = Self::container(&root, false)? else {
            return Ok(None);
        };
        let Some(prop) = container.get(HUB_NAME) else {
            return Ok(None);
        };
        prop.remove();
        Ok(Some(root.to_string()))
    }

    fn state(&self, text: &str, entry: &Entry) -> Result<State> {
        let root = Self::parse(text)?;
        let Some(container) = Self::container(&root, false)? else {
            return Ok(State::Missing);
        };
        let Some(existing) = Self::entry(&container) else {
            return Ok(State::Missing);
        };
        let existing = existing.to_serde_value().unwrap_or(Value::Null);
        let current = Self::wanted(entry)
            .iter()
            .all(|(key, want)| existing.get(key) == Some(want));
        Ok(if current {
            State::Current
        } else {
            State::Outdated
        })
    }
}

// ---- files -----------------------------------------------------------------------------------

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(nativemem_core::error::Error::io(path, e).into()),
    }
}

fn text_of(path: &Path, bytes: Option<&[u8]>) -> Result<Option<String>> {
    bytes
        .map(|b| {
            String::from_utf8(b.to_vec())
                .map_err(|_| parse_error(&format!("{} is not UTF-8 text", path.display())))
        })
        .transpose()
}

/// Read, transform, and write back only when the file is unchanged since the read. A file that
/// changed (Claude and Codex rewrite their files while running) is read again, up to `ATTEMPTS`
/// times. The previous file is backed up before it is replaced. `between` runs after the
/// transform and before the file is checked again.
fn update(
    agent: &'static str,
    path: &Path,
    backups: &Backups,
    transform: &dyn Fn(Option<&str>) -> Result<Option<String>>,
    between: &mut dyn FnMut(usize),
) -> Result<bool> {
    for attempt in 0..ATTEMPTS {
        let before = read(path)?;
        let text = text_of(path, before.as_deref())?;
        let Some(updated) = transform(text.as_deref()).map_err(|e| located(path, e))? else {
            return Ok(false);
        };
        between(attempt);
        if read(path)? != before {
            continue;
        }
        if before.is_some() {
            let root = path.parent().unwrap_or(Path::new("."));
            backup(
                backups.session,
                backups.state,
                &AgentId::new(agent),
                root,
                path,
            )?;
        }
        atomic_write(path, updated.as_bytes())?;
        return Ok(true);
    }
    Err(HubError::Busy(format!(
        "{} kept changing; nothing was written",
        path.display()
    )))
}

fn json_format(agent: &str) -> JsonServers {
    JsonServers {
        claude: agent == "claude",
    }
}

/// The agent's config files in the order they are considered, and its format.
fn files(agent: &str, roots: &AgentRoots) -> (Vec<PathBuf>, Box<dyn Format>) {
    match agent {
        "claude" => (
            vec![claude_config_path(roots.claude)],
            Box::new(json_format("claude")),
        ),
        "codex" => (vec![roots.codex.join("config.toml")], Box::new(CodexToml)),
        "opencode" => (
            vec![
                roots.opencode.join("opencode.json"),
                roots.opencode.join("opencode.jsonc"),
            ],
            Box::new(OpenCodeJsonc),
        ),
        _ => (vec![roots.pi.join("mcp.json")], Box::new(json_format("pi"))),
    }
}

fn home<'a>(agent: &str, roots: &AgentRoots<'a>) -> &'a Path {
    match agent {
        "claude" => roots.claude,
        "codex" => roots.codex,
        "opencode" => roots.opencode,
        _ => roots.pi,
    }
}

/// The file an install writes: the one that already holds the entry, else the first that exists,
/// else the first candidate (it is created).
fn install_target(files: &[PathBuf], format: &dyn Format, entry: &Entry) -> Result<PathBuf> {
    let mut first_existing = None;
    for file in files {
        let Some(bytes) = read(file)? else { continue };
        let text = text_of(file, Some(&bytes))?.unwrap_or_default();
        if !text.trim().is_empty() {
            let state = format.state(&text, entry).map_err(|e| located(file, e))?;
            if state != State::Missing {
                return Ok(file.clone());
            }
        }
        first_existing.get_or_insert_with(|| file.clone());
    }
    Ok(first_existing.unwrap_or_else(|| files[0].clone()))
}

fn selected(only: Option<&str>) -> Result<Vec<&'static str>> {
    match only {
        None => Ok(AGENTS.to_vec()),
        Some(id) => AGENTS
            .iter()
            .find(|a| **a == id)
            .map(|a| vec![*a])
            .ok_or_else(|| {
                HubError::Invalid(format!(
                    "unknown agent {id:?} (expected claude, codex, opencode or pi)"
                ))
            }),
    }
}

/// Register the hub in the agents (all of them, or only `only`). An agent whose home directory is
/// missing is skipped as `not_installed`.
pub fn install(
    roots: &AgentRoots,
    only: Option<&str>,
    entry: &Entry,
    backups: &Backups,
) -> Result<Vec<AgentOutcome>> {
    install_with(roots, only, entry, backups, &mut |_| {})
}

fn install_with(
    roots: &AgentRoots,
    only: Option<&str>,
    entry: &Entry,
    backups: &Backups,
    between: &mut dyn FnMut(usize),
) -> Result<Vec<AgentOutcome>> {
    let mut out = Vec::new();
    for agent in selected(only)? {
        if !home(agent, roots).is_dir() {
            out.push(AgentOutcome::new(agent, "not_installed", None));
            continue;
        }
        let (candidates, format) = files(agent, roots);
        let outcome = (|| -> Result<AgentOutcome> {
            let path = install_target(&candidates, format.as_ref(), entry)?;
            let changed = update(
                agent,
                &path,
                backups,
                &|text| format.install(text, entry),
                between,
            )?;
            let mut outcome = AgentOutcome::new(
                agent,
                if changed { "installed" } else { "unchanged" },
                Some(path),
            );
            if agent == "pi" {
                outcome.warnings = pi_warnings(roots.pi);
            }
            Ok(outcome)
        })();
        out.push(
            outcome.unwrap_or_else(|e| AgentOutcome::failed(agent, candidates.first().cloned(), e)),
        );
    }
    Ok(out)
}

/// Remove the `nativemem` entry from the agents. Nothing else is touched.
pub fn uninstall(
    roots: &AgentRoots,
    only: Option<&str>,
    backups: &Backups,
) -> Result<Vec<AgentOutcome>> {
    let mut out = Vec::new();
    for agent in selected(only)? {
        if !home(agent, roots).is_dir() {
            out.push(AgentOutcome::new(agent, "not_installed", None));
            continue;
        }
        let (candidates, format) = files(agent, roots);
        let mut removed_from = None;
        let mut failure = None;
        for path in &candidates {
            let result = update(
                agent,
                path,
                backups,
                &|text| match text {
                    None => Ok(None),
                    Some(t) => format.uninstall(t),
                },
                &mut |_| {},
            );
            match result {
                Ok(true) => removed_from = Some(path.clone()),
                Ok(false) => {}
                Err(e) => {
                    failure = Some((path.clone(), e));
                    break;
                }
            }
        }
        out.push(match (failure, removed_from) {
            (Some((path, e)), _) => AgentOutcome::failed(agent, Some(path), e),
            (None, Some(path)) => AgentOutcome::new(agent, "removed", Some(path)),
            (None, None) => AgentOutcome::new(agent, "absent", candidates.first().cloned()),
        });
    }
    Ok(out)
}

/// Whether the agents hold the entry, and whether it is what `install` would write now.
pub fn status(roots: &AgentRoots, only: Option<&str>, entry: &Entry) -> Result<Vec<AgentOutcome>> {
    let mut out = Vec::new();
    for agent in selected(only)? {
        if !home(agent, roots).is_dir() {
            out.push(AgentOutcome::new(agent, "not_installed", None));
            continue;
        }
        let (candidates, format) = files(agent, roots);
        let checked = (|| -> Result<(State, Option<PathBuf>)> {
            for path in &candidates {
                let Some(bytes) = read(path)? else { continue };
                let text = text_of(path, Some(&bytes))?.unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                let state = format.state(&text, entry).map_err(|e| located(path, e))?;
                if state != State::Missing {
                    return Ok((state, Some(path.clone())));
                }
            }
            Ok((State::Missing, None))
        })();
        out.push(match checked {
            Ok((state, path)) => {
                let status = match state {
                    State::Missing => "missing",
                    State::Current => "registered",
                    State::Outdated => "outdated",
                };
                let mut outcome =
                    AgentOutcome::new(agent, status, path.or_else(|| candidates.first().cloned()));
                if agent == "pi" {
                    outcome.warnings = pi_warnings(roots.pi);
                }
                outcome
            }
            Err(e) => AgentOutcome::failed(agent, candidates.first().cloned(), e),
        });
    }
    Ok(out)
}

// ---- Pi: an extension that replaces the built-in MCP support -------------------------------------

/// Whether the source text registers the slash command `mcp`.
fn registers_mcp_command(source: &str) -> bool {
    let mut rest = source;
    while let Some(at) = rest.find("registerCommand(") {
        rest = &rest[at + "registerCommand(".len()..];
        let argument = rest.trim_start();
        for quote in ['"', '\'', '`'] {
            if let Some(after) = argument.strip_prefix(quote) {
                let name = after.strip_prefix('/').unwrap_or(after);
                if name
                    .strip_prefix("mcp")
                    .is_some_and(|r| r.starts_with(quote))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// The source files of the extensions Pi discovers below its home (one level deep) and the ones
/// listed in `settings.json`.
fn pi_extension_sources(pi_home: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(pi_home.join("extensions")) {
        entries.extend(dir.flatten().map(|e| e.path()));
    }
    if let Some(settings) = read(&pi_home.join("settings.json"))
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        for listed in settings["extensions"].as_array().into_iter().flatten() {
            if let Some(p) = listed.as_str() {
                let path = match p.strip_prefix("~/") {
                    Some(rest) => std::env::home_dir().map(|h| h.join(rest)),
                    None => Some(pi_home.join(p)),
                };
                entries.extend(path);
            }
        }
    }
    let mut sources = Vec::new();
    for entry in entries {
        if entry.is_dir() {
            for index in ["index.ts", "index.js"] {
                sources.push(entry.join(index));
            }
        } else {
            sources.push(entry);
        }
    }
    sources
}

fn pi_warnings(pi_home: &Path) -> Vec<String> {
    for source in pi_extension_sources(pi_home) {
        let is_script = source
            .extension()
            .is_some_and(|e| e == "ts" || e == "js" || e == "mjs");
        if !is_script {
            continue;
        }
        if std::fs::read_to_string(&source).is_ok_and(|text| registers_mcp_command(&text)) {
            return vec![format!(
                "the Pi extension {} registers /mcp and replaces Pi's built-in MCP support: Pi does not read mcp.json while it is installed",
                source.display()
            )];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            command: "/opt/nativemem".into(),
            args: vec!["mcp".into(), "serve".into()],
        }
    }

    #[test]
    fn indentation_follows_the_file() {
        assert_eq!(indent_of("{\n    \"a\": 1\n}"), "    ");
        assert_eq!(indent_of("{\n\t\"a\": 1\n}"), "\t");
        assert_eq!(indent_of("{}"), "  ");
    }

    #[test]
    fn extension_sources_that_register_mcp_are_recognised() {
        assert!(registers_mcp_command("pi.registerCommand(\"mcp\", {})"));
        assert!(registers_mcp_command("pi.registerCommand( '/mcp', {})"));
        assert!(registers_mcp_command("x.registerCommand(`mcp`, h)"));
        assert!(!registers_mcp_command(
            "pi.registerCommand(\"mcp-tools\", {})"
        ));
        assert!(!registers_mcp_command("pi.registerCommand(\"other\", {})"));
    }

    #[test]
    fn claude_update_starts_over_when_the_file_changes_under_it() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join(".claude.json");
        std::fs::write(&path, "{\n  \"theme\": \"dark\"\n}\n").unwrap();
        let state = t.path().join("state");
        let session = BackupSession::with_stamp("s");
        let backups = Backups {
            state: &state,
            session: &session,
        };
        let format = json_format("claude");
        let mut calls = Vec::new();
        let changed = update(
            "claude",
            &path,
            &backups,
            &|text| format.install(text, &entry()),
            &mut |attempt| {
                calls.push(attempt);
                if attempt == 0 {
                    // Claude writes its own change after we read.
                    std::fs::write(&path, "{\n  \"theme\": \"light\",\n  \"tips\": 3\n}\n")
                        .unwrap();
                }
            },
        )
        .unwrap();
        assert!(changed);
        assert_eq!(calls, [0, 1]);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["theme"], "light", "the concurrent change survives");
        assert_eq!(written["tips"], 3);
        assert_eq!(
            written["mcpServers"]["nativemem"]["command"],
            "/opt/nativemem"
        );
        // The backup holds the file as it was when the write went ahead.
        let saved = state.join("backups/s/claude/.claude.json");
        assert!(std::fs::read_to_string(saved).unwrap().contains("light"));
    }

    #[test]
    fn a_file_that_never_stops_changing_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join(".claude.json");
        std::fs::write(&path, "{}\n").unwrap();
        let state = t.path().join("state");
        let session = BackupSession::with_stamp("s");
        let backups = Backups {
            state: &state,
            session: &session,
        };
        let format = json_format("claude");
        let mut n = 0;
        let error = update(
            "claude",
            &path,
            &backups,
            &|text| format.install(text, &entry()),
            &mut |_| {
                n += 1;
                std::fs::write(&path, format!("{{\"n\": {n}}}\n")).unwrap();
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "busy");
        assert_eq!(n, ATTEMPTS);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("nativemem"));
    }
}
