//! How each harness is started headless and how its answer is read back. One command builder per
//! harness, one parser per output shape. Nothing here starts a process.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use nativemem_core::probe::resolve_executable;
use serde_json::Value;

use crate::subagents::{Harness, SubagentSpec};

/// A harness invocation, ready to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The prompt, when the harness reads it from stdin.
    pub stdin: Option<String>,
    /// The file the harness writes its answer to, when it does not print it.
    pub answer_file: Option<PathBuf>,
}

/// `PATH` plus the directories the agents install their binaries into. Agents start the hub with a
/// minimal `PATH`, so the process `PATH` alone would not find them.
pub fn search_path() -> OsString {
    let mut dirs: Vec<PathBuf> = nativemem_core::probe::process_path()
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::home_dir();
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    if let Some(home) = &home {
        dirs.push(home.join(".pi/agent/bin"));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// The absolute path of the harness binary in `search_path`, symlinks resolved. It is only located.
pub fn locate(harness: Harness, search_path: &OsStr) -> Option<PathBuf> {
    resolve_executable(harness.binary(), Some(search_path))
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

/// The command for one run. `scratch` is a directory private to the run, for the harnesses that
/// write their answer to a file.
pub fn build(program: &Path, spec: &SubagentSpec, prompt: &str, scratch: &Path) -> Invocation {
    let prompt = spec.compose_prompt(prompt);
    let mut args: Vec<OsString> = Vec::new();
    let extra: Vec<&str> = spec.args.iter().map(String::as_str).collect();
    let mut stdin = None;
    let mut answer_file = None;
    match spec.harness {
        Harness::Claude => {
            push(
                &mut args,
                &["-p", "--output-format", "json", "--model", &spec.model],
            );
            push(&mut args, &extra);
            stdin = Some(prompt);
        }
        Harness::Codex => {
            let file = scratch.join("answer.txt");
            push(
                &mut args,
                &["exec", "--model", &spec.model, "--skip-git-repo-check"],
            );
            push(&mut args, &extra);
            args.push("-o".into());
            args.push(file.clone().into());
            // `-` makes Codex read the prompt from stdin.
            args.push("-".into());
            answer_file = Some(file);
            stdin = Some(prompt);
        }
        Harness::Opencode => {
            push(
                &mut args,
                &["run", "--model", &spec.model, "--format", "json"],
            );
            push(&mut args, &extra);
            // A prompt that starts with a dash would read as a flag.
            args.push(if prompt.starts_with('-') {
                format!("\n{prompt}").into()
            } else {
                prompt.into()
            });
        }
        Harness::Pi => {
            push(&mut args, &["-p", "--model", &spec.model]);
            push(&mut args, &extra);
            // `--` ends option parsing, so the prompt cannot read as a flag.
            push(&mut args, &["--"]);
            args.push(prompt.into());
        }
    }
    Invocation {
        program: program.to_path_buf(),
        args,
        stdin,
        answer_file,
    }
}

/// The answer from a finished run: `stdout` and, for a harness with an answer file, its content.
/// The error is a short description, never the output itself.
pub fn parse_answer(
    harness: Harness,
    stdout: &str,
    answer_file: Option<&str>,
) -> Result<String, String> {
    let text = match harness {
        Harness::Claude => parse_claude(stdout)?,
        Harness::Codex => answer_file
            .ok_or("codex wrote no answer file")?
            .trim_end()
            .to_string(),
        Harness::Opencode => parse_opencode(stdout),
        Harness::Pi => stdout.trim_end().to_string(),
    };
    if text.trim().is_empty() {
        Err(format!("{} returned no answer", harness.as_str()))
    } else {
        Ok(text)
    }
}

/// `claude -p --output-format json` prints one object whose `result` is the answer; an error run
/// carries `is_error: true` and the message in `result`.
fn parse_claude(stdout: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "claude printed no JSON document".to_string())?;
    let result = match &value {
        Value::Array(events) => events
            .iter()
            .rev()
            .find(|e| e["type"] == "result")
            .cloned()
            .unwrap_or(Value::Null),
        other => other.clone(),
    };
    let text = result["result"]
        .as_str()
        .ok_or("claude's JSON document has no result")?;
    if result["is_error"] == true {
        return Err(format!("claude reported an error: {text}"));
    }
    Ok(text.to_string())
}

/// `opencode run --format json` prints one JSON event per line; the `text` events carry the answer
/// parts. [UNVERIFIED] shape: `{"type":"text","part":{"text":...}}`.
fn parse_opencode(stdout: &str) -> String {
    let parts: Vec<String> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "text")
        .filter_map(|event| {
            event["part"]["text"]
                .as_str()
                .or_else(|| event["text"].as_str())
                .map(str::to_string)
        })
        .collect();
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(harness: Harness) -> SubagentSpec {
        let mut s = SubagentSpec::new(harness, "the-model", "d");
        s.args = vec!["--extra".into(), "x y".into()];
        s.instructions = Some("Rules.".into());
        s
    }

    fn strings(invocation: &Invocation) -> Vec<String> {
        invocation
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_reads_the_prompt_from_stdin() {
        let i = build(
            Path::new("/bin/claude"),
            &spec(Harness::Claude),
            "hi",
            Path::new("/scratch"),
        );
        assert_eq!(i.program, Path::new("/bin/claude"));
        assert_eq!(
            strings(&i),
            [
                "-p",
                "--output-format",
                "json",
                "--model",
                "the-model",
                "--extra",
                "x y"
            ]
        );
        assert_eq!(i.stdin.as_deref(), Some("Rules.\n\nhi"));
        assert_eq!(i.answer_file, None);
    }

    #[test]
    fn codex_reads_stdin_and_writes_its_answer_to_a_file() {
        let i = build(
            Path::new("/bin/codex"),
            &spec(Harness::Codex),
            "hi",
            Path::new("/scratch"),
        );
        assert_eq!(
            strings(&i),
            [
                "exec",
                "--model",
                "the-model",
                "--skip-git-repo-check",
                "--extra",
                "x y",
                "-o",
                "/scratch/answer.txt",
                "-"
            ]
        );
        assert_eq!(i.stdin.as_deref(), Some("Rules.\n\nhi"));
        assert_eq!(
            i.answer_file.as_deref(),
            Some(Path::new("/scratch/answer.txt"))
        );
    }

    #[test]
    fn opencode_takes_the_prompt_as_the_last_argument() {
        let i = build(
            Path::new("/bin/opencode"),
            &spec(Harness::Opencode),
            "hi",
            Path::new("/s"),
        );
        assert_eq!(
            strings(&i),
            [
                "run",
                "--model",
                "the-model",
                "--format",
                "json",
                "--extra",
                "x y",
                "Rules.\n\nhi"
            ]
        );
        assert_eq!(i.stdin, None);
        let mut dash = spec(Harness::Opencode);
        dash.instructions = None;
        let i = build(Path::new("/bin/opencode"), &dash, "- item", Path::new("/s"));
        assert_eq!(strings(&i).last().unwrap(), "\n- item");
    }

    #[test]
    fn pi_takes_the_prompt_after_a_double_dash() {
        let i = build(
            Path::new("/bin/pi"),
            &spec(Harness::Pi),
            "hi",
            Path::new("/s"),
        );
        assert_eq!(
            strings(&i),
            [
                "-p",
                "--model",
                "the-model",
                "--extra",
                "x y",
                "--",
                "Rules.\n\nhi"
            ]
        );
        assert_eq!(i.stdin, None);
    }

    #[test]
    fn claude_answer_is_the_result_field() {
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"done\n","session_id":"s"}"#;
        assert_eq!(parse_answer(Harness::Claude, ok, None).unwrap(), "done\n");
        let events = r#"[{"type":"system"},{"type":"result","is_error":false,"result":"last"}]"#;
        assert_eq!(parse_answer(Harness::Claude, events, None).unwrap(), "last");
        let error = r#"{"type":"result","is_error":true,"result":"rate limited"}"#;
        assert!(parse_answer(Harness::Claude, error, None)
            .unwrap_err()
            .contains("rate limited"));
        assert!(parse_answer(Harness::Claude, "not json", None).is_err());
        assert!(parse_answer(Harness::Claude, r#"{"type":"result"}"#, None).is_err());
        assert!(parse_answer(Harness::Claude, r#"{"result":""}"#, None).is_err());
    }

    #[test]
    fn codex_answer_is_the_file() {
        assert_eq!(
            parse_answer(Harness::Codex, "progress noise", Some("final\n\n")).unwrap(),
            "final"
        );
        assert!(parse_answer(Harness::Codex, "x", None).is_err());
        assert!(parse_answer(Harness::Codex, "x", Some("  \n")).is_err());
    }

    #[test]
    fn opencode_answer_is_the_text_events() {
        let out = concat!(
            "{\"type\":\"step_start\",\"part\":{\"type\":\"step-start\"}}\n",
            "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"first\"}}\n",
            "not json\n",
            "{\"type\":\"tool_use\",\"part\":{\"text\":\"ignored\"}}\n",
            "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"second\"}}\n",
            "{\"type\":\"step_finish\"}\n",
        );
        assert_eq!(
            parse_answer(Harness::Opencode, out, None).unwrap(),
            "first\n\nsecond"
        );
        assert!(parse_answer(Harness::Opencode, "{\"type\":\"step_finish\"}\n", None).is_err());
    }

    #[test]
    fn pi_answer_is_stdout() {
        assert_eq!(
            parse_answer(Harness::Pi, "answer\n\n", None).unwrap(),
            "answer"
        );
        assert!(parse_answer(Harness::Pi, "\n", None).is_err());
    }

    #[test]
    fn the_binary_is_found_in_the_given_directories_only() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("pi"), "").unwrap();
        let found = locate(Harness::Pi, t.path().as_os_str()).unwrap();
        assert_eq!(found, t.path().join("pi").canonicalize().unwrap());
        assert_eq!(locate(Harness::Claude, t.path().as_os_str()), None);
    }

    #[test]
    fn the_search_path_adds_the_install_directories_after_path() {
        let dirs: Vec<PathBuf> = std::env::split_paths(&search_path()).collect();
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        if let Some(home) = std::env::home_dir() {
            assert!(dirs.contains(&home.join(".local/bin")));
            assert!(dirs.contains(&home.join(".pi/agent/bin")));
        }
    }
}
