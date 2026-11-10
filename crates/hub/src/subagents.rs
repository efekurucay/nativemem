//! Subagent definitions: `<state dir>/subagents.toml`. A subagent is a coding agent run headless
//! with a chosen harness and model; the hub exposes each definition as a tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nativemem_core::fsx::{atomic_write, backup, BackupSession};
use nativemem_core::model::AgentId;
use serde::{Deserialize, Serialize};

use crate::error::{HubError, Result};

pub const SUBAGENTS_FILE: &str = "subagents.toml";

/// Backups of `subagents.toml` use the hub's agent directory:
/// `<state dir>/backups/<stamp>/hub/subagents.toml`.
const BACKUP_AGENT: &str = "hub";

const MAX_NAME: usize = 48;

pub const DEFAULT_TIMEOUT_SECS: u64 = 900;

pub fn subagents_path(state: &Path) -> PathBuf {
    state.join(SUBAGENTS_FILE)
}

/// The coding agents a subagent can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Opencode,
    Pi,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::Pi => "pi",
        }
    }

    /// The executable name; it equals the harness name for every harness.
    pub fn binary(self) -> &'static str {
        self.as_str()
    }
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// One subagent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentSpec {
    pub harness: Harness,
    /// A model id in the harness's own syntax.
    pub model: String,
    /// When to use it; it becomes the tool description.
    pub description: String,
    /// Text put in front of every prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Extra harness flags, placed before the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl SubagentSpec {
    pub fn new(harness: Harness, model: &str, description: &str) -> Self {
        SubagentSpec {
            harness,
            model: model.to_string(),
            description: description.to_string(),
            instructions: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            args: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(HubError::Invalid(m.to_string()));
        if self.model.trim().is_empty() {
            return invalid("a subagent needs a model");
        }
        // The model is passed as the value of `--model`; a leading dash would read as a flag.
        if self.model.starts_with('-') || self.model.chars().any(char::is_control) {
            return invalid("the model must not start with - or hold control characters");
        }
        if self.description.trim().is_empty() {
            return invalid("a subagent needs a description");
        }
        if self.timeout_secs == 0 {
            return invalid("timeout_secs must be at least 1");
        }
        Ok(())
    }

    /// The prompt the harness receives: the instructions, a blank line, then the caller's prompt.
    pub fn compose_prompt(&self, prompt: &str) -> String {
        match self
            .instructions
            .as_deref()
            .filter(|i| !i.trim().is_empty())
        {
            Some(instructions) => format!("{}\n\n{prompt}", instructions.trim_end()),
            None => prompt.to_string(),
        }
    }
}

/// A subagent name is also the suffix of its tool name `subagent_<name>`.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(HubError::Invalid(format!(
            "subagent name {name:?} must be 1-{MAX_NAME} characters of a-z 0-9 -"
        )))
    }
}

/// One subagent as a JSON document: the spec's fields, plus an optional `name`.
pub fn parse_spec_json(input: &str) -> Result<(Option<String>, SubagentSpec)> {
    let mut value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| HubError::Invalid(format!("the subagent spec is not valid JSON: {e}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| HubError::Invalid("the subagent spec must be a JSON object".into()))?;
    let name = match object.remove("name") {
        None => None,
        Some(serde_json::Value::String(n)) => Some(n),
        Some(_) => return Err(HubError::Invalid("name must be a string".into())),
    };
    let spec: SubagentSpec = serde_json::from_value(value)
        .map_err(|e| HubError::Invalid(format!("invalid subagent spec: {e}")))?;
    spec.validate()?;
    Ok((name, spec))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentConfig {
    #[serde(default)]
    pub agents: BTreeMap<String, SubagentSpec>,
}

impl SubagentConfig {
    pub fn agent(&self, name: &str) -> Result<&SubagentSpec> {
        self.agents
            .get(name)
            .ok_or_else(|| HubError::NotFound(format!("no subagent named {name:?}")))
    }

    /// A missing file is an empty config.
    pub fn load(state: &Path) -> Result<Self> {
        let path = subagents_path(state);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SubagentConfig::default())
            }
            Err(e) => return Err(nativemem_core::error::Error::io(&path, e).into()),
        };
        let config: SubagentConfig = toml::from_str(&text)
            .map_err(|e| HubError::Parse(format!("cannot parse {}: {e}", path.display())))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        for (name, spec) in &self.agents {
            validate_name(name)?;
            spec.validate()
                .map_err(|e| HubError::Invalid(format!("subagent {name:?}: {e}")))?;
        }
        Ok(())
    }

    /// Back up the existing file, then replace it atomically. The caller holds the state lock.
    pub fn save(&self, state: &Path, backups: &BackupSession) -> Result<()> {
        self.validate()?;
        let text = toml::to_string_pretty(self)
            .map_err(|e| HubError::Parse(format!("cannot encode the subagent config: {e}")))?;
        let path = subagents_path(state);
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

    fn sample() -> SubagentConfig {
        let mut reviewer = SubagentSpec::new(Harness::Claude, "sonnet", "Reviews a diff");
        reviewer.instructions = Some("Be terse.".into());
        reviewer.timeout_secs = 60;
        reviewer.args = vec!["--allowed-tools".into(), "Read".into()];
        let mut config = SubagentConfig::default();
        config.agents.insert("reviewer".into(), reviewer);
        config.agents.insert(
            "scout".into(),
            SubagentSpec::new(Harness::Opencode, "anthropic/claude-haiku", "Finds code"),
        );
        config
    }

    #[test]
    fn config_round_trips_and_backs_up_the_previous_file() {
        let t = tempfile::tempdir().unwrap();
        let state = t.path().join("state");
        let config = sample();
        let backups = BackupSession::with_stamp("20260101T000000Z");
        config.save(&state, &backups).unwrap();
        assert_eq!(SubagentConfig::load(&state).unwrap(), config);
        assert!(!state.join("backups").exists());

        let mut changed = config.clone();
        changed.agents.remove("scout");
        changed.save(&state, &backups).unwrap();
        assert_eq!(SubagentConfig::load(&state).unwrap(), changed);
        let saved = state.join("backups/20260101T000000Z/hub/subagents.toml");
        let old: SubagentConfig = toml::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
        assert_eq!(old, config);
    }

    #[test]
    fn written_file_has_the_documented_shape() {
        let t = tempfile::tempdir().unwrap();
        sample()
            .save(t.path(), &BackupSession::with_stamp("s"))
            .unwrap();
        let text = std::fs::read_to_string(subagents_path(t.path())).unwrap();
        assert!(text.contains("[agents.reviewer]"), "{text}");
        assert!(text.contains("harness = \"claude\""), "{text}");
        assert!(text.contains("timeout_secs = 900"), "{text}");
        assert!(text.contains("instructions = \"Be terse.\""), "{text}");
    }

    #[test]
    fn missing_file_is_an_empty_config() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            SubagentConfig::load(t.path()).unwrap(),
            SubagentConfig::default()
        );
    }

    #[test]
    fn names_models_and_unknown_keys_are_checked() {
        for ok in ["a", "code-review", &"a".repeat(48)] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Upper", "under_score", "a b", &"a".repeat(49)] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        let mut s = SubagentSpec::new(Harness::Pi, "m", "d");
        assert!(s.validate().is_ok());
        s.model = "-x".into();
        assert!(s.validate().is_err());
        s.model = " ".into();
        assert!(s.validate().is_err());
        let mut s = SubagentSpec::new(Harness::Pi, "m", "d");
        s.timeout_secs = 0;
        assert!(s.validate().is_err());
        let mut s = SubagentSpec::new(Harness::Pi, "m", " ");
        assert!(s.validate().is_err());
        s.description = "d".into();
        assert!(s.validate().is_ok());
        let unknown =
            "[agents.a]\nharness = \"pi\"\nmodel = \"m\"\ndescription = \"d\"\nbogus = 1\n";
        assert!(toml::from_str::<SubagentConfig>(unknown).is_err());
        let unknown_harness =
            "[agents.a]\nharness = \"hermes\"\nmodel = \"m\"\ndescription = \"d\"\n";
        assert!(toml::from_str::<SubagentConfig>(unknown_harness).is_err());
    }

    #[test]
    fn spec_json_carries_an_optional_name_and_defaults() {
        let (name, spec) =
            parse_spec_json(r#"{"name":"a","harness":"codex","model":"gpt-5","description":"d"}"#)
                .unwrap();
        assert_eq!(name.as_deref(), Some("a"));
        assert_eq!(spec, SubagentSpec::new(Harness::Codex, "gpt-5", "d"));
        assert!(parse_spec_json("[]").is_err());
        assert!(parse_spec_json(r#"{"harness":"pi"}"#).is_err());
        assert!(
            parse_spec_json(r#"{"harness":"pi","model":"m","description":"d","bogus":1}"#).is_err()
        );
    }

    #[test]
    fn the_prompt_gets_the_instructions_in_front() {
        let mut s = SubagentSpec::new(Harness::Pi, "m", "d");
        assert_eq!(s.compose_prompt("go"), "go");
        s.instructions = Some("Be brief.\n".into());
        assert_eq!(s.compose_prompt("go"), "Be brief.\n\ngo");
        s.instructions = Some("  ".into());
        assert_eq!(s.compose_prompt("go"), "go");
    }
}
