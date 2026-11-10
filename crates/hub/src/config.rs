use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nativemem_core::fsx::{atomic_write, backup, BackupSession};
use nativemem_core::model::AgentId;
use serde::{Deserialize, Serialize};

use crate::error::{HubError, Result};
use crate::secrets::{secret_ref, validate_secret_id};

pub const CONFIG_FILE: &str = "hub.toml";

/// The name the hub is registered under in the agents. A server with this name would be the hub
/// calling itself.
pub const HUB_NAME: &str = "nativemem";

/// Backups of `hub.toml` use this agent directory: `<state dir>/backups/<stamp>/hub/hub.toml`.
const BACKUP_AGENT: &str = "hub";

pub fn config_path(state: &Path) -> PathBuf {
    state.join(CONFIG_FILE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    #[default]
    None,
    Oauth,
}

impl Auth {
    pub fn as_str(self) -> &'static str {
        match self {
            Auth::None => "none",
            Auth::Oauth => "oauth",
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

/// One upstream MCP server. Any string value may be `secret:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSpec {
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl ServerSpec {
    pub fn stdio(command: impl Into<String>) -> Self {
        ServerSpec {
            transport: Transport::Stdio,
            command: Some(command.into()),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            auth: Auth::None,
            enabled: true,
        }
    }

    pub fn http(url: impl Into<String>) -> Self {
        ServerSpec {
            transport: Transport::Http,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            url: Some(url.into()),
            headers: BTreeMap::new(),
            auth: Auth::None,
            enabled: true,
        }
    }

    /// Every string value of the spec, in a fixed order.
    fn values(&self) -> impl Iterator<Item = &str> {
        self.command
            .iter()
            .map(String::as_str)
            .chain(self.args.iter().map(String::as_str))
            .chain(self.env.values().map(String::as_str))
            .chain(self.cwd.iter().map(String::as_str))
            .chain(self.url.iter().map(String::as_str))
            .chain(self.headers.values().map(String::as_str))
    }

    /// The secret ids the spec refers to, sorted, without duplicates.
    pub fn secret_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .values()
            .filter_map(secret_ref)
            .map(str::to_string)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(HubError::Invalid(m.to_string()));
        match self.transport {
            Transport::Stdio => {
                match self.command.as_deref() {
                    None | Some("") => return invalid("a stdio server needs a command"),
                    Some(_) => {}
                }
                if self.url.is_some() || !self.headers.is_empty() {
                    return invalid("a stdio server takes no url or headers");
                }
                if self.auth != Auth::None {
                    return invalid("auth applies to http servers only");
                }
                if let Some(cwd) = self.cwd.as_deref() {
                    if secret_ref(cwd).is_none() && !Path::new(cwd).is_absolute() {
                        return invalid("cwd must be an absolute path");
                    }
                }
            }
            Transport::Http => {
                match self.url.as_deref() {
                    None | Some("") => return invalid("an http server needs a url"),
                    Some(u)
                        if secret_ref(u).is_none()
                            && !(u.starts_with("http://") || u.starts_with("https://")) =>
                    {
                        return invalid("url must start with http:// or https://");
                    }
                    Some(_) => {}
                }
                if self.command.is_some()
                    || !self.args.is_empty()
                    || !self.env.is_empty()
                    || self.cwd.is_some()
                {
                    return invalid("an http server takes no command, args, env or cwd");
                }
                if self.auth == Auth::Oauth
                    && self
                        .headers
                        .keys()
                        .any(|k| k.eq_ignore_ascii_case("authorization"))
                {
                    return invalid("an oauth server takes no Authorization header");
                }
            }
        }
        for id in self.secret_ids() {
            validate_secret_id(&id)?;
        }
        Ok(())
    }
}

/// One server as a JSON document: the spec's fields, plus an optional `name`.
pub fn parse_spec_json(input: &str) -> Result<(Option<String>, ServerSpec)> {
    let mut value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| HubError::Invalid(format!("the server spec is not valid JSON: {e}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| HubError::Invalid("the server spec must be a JSON object".into()))?;
    let name = match object.remove("name") {
        None => None,
        Some(serde_json::Value::String(n)) => Some(n),
        Some(_) => return Err(HubError::Invalid("name must be a string".into())),
    };
    let spec: ServerSpec = serde_json::from_value(value)
        .map_err(|e| HubError::Invalid(format!("invalid server spec: {e}")))?;
    spec.validate()?;
    Ok((name, spec))
}

/// A server name is also the prefix of its tool names, so it stays within the characters a tool
/// name may hold.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !ok {
        return Err(HubError::Invalid(format!(
            "server name {name:?} must be 1-32 characters of A-Z a-z 0-9 - _"
        )));
    }
    if name == HUB_NAME {
        return Err(HubError::Invalid(format!(
            "server name {HUB_NAME:?} is the hub itself"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, ServerSpec>,
}

impl HubConfig {
    pub fn server(&self, name: &str) -> Result<&ServerSpec> {
        self.servers
            .get(name)
            .ok_or_else(|| HubError::NotFound(format!("no server named {name:?}")))
    }

    pub fn server_mut(&mut self, name: &str) -> Result<&mut ServerSpec> {
        self.servers
            .get_mut(name)
            .ok_or_else(|| HubError::NotFound(format!("no server named {name:?}")))
    }

    /// A missing file is an empty config.
    pub fn load(state: &Path) -> Result<Self> {
        let path = config_path(state);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HubConfig::default()),
            Err(e) => return Err(nativemem_core::error::Error::io(&path, e).into()),
        };
        let config: HubConfig = toml::from_str(&text)
            .map_err(|e| HubError::Parse(format!("cannot parse {}: {e}", path.display())))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        for (name, spec) in &self.servers {
            validate_name(name)?;
            spec.validate()
                .map_err(|e| HubError::Invalid(format!("server {name:?}: {e}")))?;
        }
        Ok(())
    }

    /// Back up the existing file, then replace it atomically. The caller holds the state lock.
    pub fn save(&self, state: &Path, backups: &BackupSession) -> Result<()> {
        self.validate()?;
        let text = toml::to_string_pretty(self)
            .map_err(|e| HubError::Parse(format!("cannot encode hub config: {e}")))?;
        let path = config_path(state);
        if path.is_file() {
            backup(backups, state, &AgentId::new(BACKUP_AGENT), state, &path)?;
        }
        atomic_write(&path, text.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HubConfig {
        let mut stdio = ServerSpec::stdio("/usr/bin/tool");
        stdio.args = vec!["--flag".into(), "secret:arg-token".into()];
        stdio.env.insert("TOKEN".into(), "secret:gh-token".into());
        stdio.cwd = Some("/work".into());
        let mut http = ServerSpec::http("https://mcp.example.com/v1");
        http.headers.insert("X-Api-Key".into(), "secret:api".into());
        http.auth = Auth::Oauth;
        http.enabled = false;
        let mut c = HubConfig::default();
        c.servers.insert("local".into(), stdio);
        c.servers.insert("remote".into(), http);
        c
    }

    #[test]
    fn config_round_trips_and_backs_up_the_previous_file() {
        let t = tempfile::tempdir().unwrap();
        let state = t.path().join("state");
        let config = sample();
        let backups = BackupSession::with_stamp("20260101T000000Z");
        config.save(&state, &backups).unwrap();
        assert_eq!(HubConfig::load(&state).unwrap(), config);
        assert!(!state.join("backups").exists());

        let mut changed = config.clone();
        changed.servers.remove("local");
        changed.save(&state, &backups).unwrap();
        assert_eq!(HubConfig::load(&state).unwrap(), changed);
        let saved = state.join("backups/20260101T000000Z/hub/hub.toml");
        let old: HubConfig = toml::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
        assert_eq!(old, config);
    }

    #[test]
    fn missing_file_is_an_empty_config() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(HubConfig::load(t.path()).unwrap(), HubConfig::default());
    }

    #[test]
    fn written_file_has_the_documented_shape() {
        let t = tempfile::tempdir().unwrap();
        sample()
            .save(t.path(), &BackupSession::with_stamp("s"))
            .unwrap();
        let text = std::fs::read_to_string(config_path(t.path())).unwrap();
        assert!(text.contains("[servers.local]"), "{text}");
        assert!(text.contains("transport = \"stdio\""), "{text}");
        assert!(text.contains("TOKEN = \"secret:gh-token\""), "{text}");
        assert!(text.contains("auth = \"oauth\""), "{text}");
    }

    #[test]
    fn unknown_keys_and_bad_specs_are_rejected() {
        let unknown = "[servers.a]\ntransport = \"stdio\"\ncommand = \"x\"\nbogus = 1\n";
        assert!(toml::from_str::<HubConfig>(unknown).is_err());
        assert!(ServerSpec::stdio("").validate().is_err());
        assert!(ServerSpec::http("ftp://x").validate().is_err());
        let mut s = ServerSpec::stdio("x");
        s.cwd = Some("relative".into());
        assert!(s.validate().is_err());
        let mut s = ServerSpec::stdio("x");
        s.env.insert("K".into(), "secret:bad id".into());
        assert!(s.validate().is_err());
        let mut s = ServerSpec::http("https://h/mcp");
        s.auth = Auth::Oauth;
        s.headers.insert("authorization".into(), "Bearer x".into());
        assert!(s.validate().is_err());
        assert!(validate_name("nativemem").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("gh_hub-2").is_ok());
    }

    #[test]
    fn spec_json_carries_an_optional_name() {
        let (name, spec) =
            parse_spec_json(r#"{"name":"a","transport":"stdio","command":"x"}"#).unwrap();
        assert_eq!(name.as_deref(), Some("a"));
        assert_eq!(spec, ServerSpec::stdio("x"));
        let (name, _) = parse_spec_json(r#"{"transport":"http","url":"http://h"}"#).unwrap();
        assert_eq!(name, None);
        assert!(parse_spec_json("[]").is_err());
        assert!(parse_spec_json(r#"{"transport":"stdio"}"#).is_err());
        assert!(parse_spec_json(r#"{"transport":"stdio","command":"x","bogus":1}"#).is_err());
    }

    #[test]
    fn secret_ids_lists_every_reference_once() {
        let mut s = ServerSpec::stdio("secret:cmd");
        s.args = vec!["secret:a".into(), "secret:a".into()];
        s.env.insert("K".into(), "secret:b".into());
        s.env.insert("L".into(), "plain".into());
        assert_eq!(s.secret_ids(), vec!["a", "b", "cmd"]);
    }
}
