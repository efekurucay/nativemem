//! Subagents against fake harness executables: shell scripts in a temp directory that print the
//! output shape of each harness and record the argv, stdin, working directory and nesting level
//! they got. No real agent is started and nothing outside the temp directory is touched.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nativemem_hub::config::HubConfig;
use nativemem_hub::runner::{Runner, MAX_ANSWER_BYTES, MAX_CONCURRENT};
use nativemem_hub::secrets::{FileStore, SecretStore};
use nativemem_hub::serve::HubServer;
use nativemem_hub::subagents::{Harness, SubagentConfig, SubagentSpec};
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

/// Records what the harness was started with, next to the script, under `$name.*`.
fn recording(name: &str) -> String {
    format!(
        r#"dir="$(dirname "$0")"
: > "$dir/{name}.argv"
for a in "$@"; do printf '%s\0' "$a" >> "$dir/{name}.argv"; done
pwd > "$dir/{name}.cwd"
printf '%s' "$NATIVEMEM_SUBAGENT_DEPTH" > "$dir/{name}.depth"
"#
    )
}

struct Fake {
    _t: tempfile::TempDir,
    bin: PathBuf,
    work: PathBuf,
}

impl Fake {
    fn new() -> Fake {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        let work = t.path().join("work");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        // The scripts run with the search path as their PATH, so the utilities they use are linked
        // in. The search path holds nothing else: a harness that has no fake is never found, so
        // no real agent can be started.
        for tool in ["cat", "sleep", "dirname"] {
            let source = ["/bin", "/usr/bin"]
                .iter()
                .map(|d| Path::new(d).join(tool))
                .find(|p| p.is_file())
                .unwrap_or_else(|| panic!("no {tool} on this system"));
            std::os::unix::fs::symlink(source, bin.join(tool)).unwrap();
        }
        Fake { _t: t, bin, work }
    }

    /// An executable `name` in the fake bin directory.
    fn script(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn recorded(&self, name: &str, what: &str) -> Vec<u8> {
        fs::read(self.bin.join(format!("{name}.{what}")))
            .unwrap_or_else(|e| panic!("{name}.{what} was not recorded: {e}"))
    }

    fn argv(&self, name: &str) -> Vec<String> {
        self.recorded(name, "argv")
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    fn ran(&self, name: &str) -> bool {
        self.bin.join(format!("{name}.argv")).exists()
    }

    fn runner(&self) -> Runner {
        Runner::from_env()
            .with_search_path(self.bin.clone().into_os_string())
            .with_depth(0)
    }
}

fn spec(harness: Harness) -> SubagentSpec {
    let mut s = SubagentSpec::new(harness, "m-1", "Does things");
    s.timeout_secs = 20;
    s
}

#[tokio::test]
async fn claude_gets_its_prompt_on_stdin_and_answers_in_the_result_field() {
    let f = Fake::new();
    f.script(
        "claude",
        &format!(
            "{}cat > \"$dir/claude.stdin\"\nprintf '%s' '{{\"type\":\"result\",\"is_error\":false,\"result\":\"claude says hi\"}}'\n",
            recording("claude")
        ),
    );
    let mut s = spec(Harness::Claude);
    s.instructions = Some("Be terse.".into());
    s.args = vec!["--allowed-tools".into(), "Read".into()];
    let report = f.runner().run(&s, "do it", Some(&f.work)).await;

    assert!(report.ok, "{report:?}");
    assert_eq!(report.text, "claude says hi");
    assert_eq!(report.exit_code, Some(0));
    assert_eq!(
        f.argv("claude"),
        [
            "-p",
            "--output-format",
            "json",
            "--model",
            "m-1",
            "--allowed-tools",
            "Read"
        ]
    );
    assert_eq!(
        String::from_utf8(f.recorded("claude", "stdin")).unwrap(),
        "Be terse.\n\ndo it"
    );
    assert_eq!(
        fs::read_to_string(f.bin.join("claude.cwd")).unwrap().trim(),
        f.work.display().to_string()
    );
    assert_eq!(f.recorded("claude", "depth"), b"1");
}

#[tokio::test]
async fn codex_gets_its_prompt_on_stdin_and_answers_in_the_file() {
    let f = Fake::new();
    f.script(
        "codex",
        &format!(
            r#"{}out=""; prev=""
for a in "$@"; do [ "$prev" = "-o" ] && out="$a"; prev="$a"; done
cat > "$dir/codex.stdin"
echo "progress noise"
printf 'codex answer\n' > "$out"
"#,
            recording("codex")
        ),
    );
    let report = f.runner().run(&spec(Harness::Codex), "do it", None).await;

    assert!(report.ok, "{report:?}");
    assert_eq!(report.text, "codex answer");
    let argv = f.argv("codex");
    assert_eq!(
        &argv[..4],
        ["exec", "--model", "m-1", "--skip-git-repo-check"]
    );
    assert_eq!(argv[4], "-o");
    assert!(argv[5].ends_with("answer.txt"), "{argv:?}");
    assert_eq!(argv[6], "-");
    assert_eq!(
        String::from_utf8(f.recorded("codex", "stdin")).unwrap(),
        "do it"
    );
    // The scratch directory is gone after the run.
    assert!(!Path::new(&argv[5]).exists());
}

#[tokio::test]
async fn opencode_gets_its_prompt_as_an_argument_and_answers_in_text_events() {
    let f = Fake::new();
    f.script(
        "opencode",
        &format!(
            r#"{}echo '{{"type":"step_start","part":{{"type":"step-start"}}}}'
echo '{{"type":"text","part":{{"type":"text","text":"part one"}}}}'
echo '{{"type":"text","part":{{"type":"text","text":"part two"}}}}'
echo '{{"type":"step_finish","part":{{"type":"step-finish"}}}}'
"#,
            recording("opencode")
        ),
    );
    let report = f
        .runner()
        .run(&spec(Harness::Opencode), "do it", None)
        .await;

    assert!(report.ok, "{report:?}");
    assert_eq!(report.text, "part one\n\npart two");
    assert_eq!(
        f.argv("opencode"),
        ["run", "--model", "m-1", "--format", "json", "do it"]
    );
}

#[tokio::test]
async fn pi_gets_its_prompt_as_an_argument_and_answers_in_plain_text() {
    let f = Fake::new();
    f.script("pi", &format!("{}echo 'pi answer'\n", recording("pi")));
    let report = f.runner().run(&spec(Harness::Pi), "do it", None).await;

    assert!(report.ok, "{report:?}");
    assert_eq!(report.text, "pi answer");
    assert_eq!(f.argv("pi"), ["-p", "--model", "m-1", "--", "do it"]);
}

#[tokio::test]
async fn a_failed_run_reports_the_exit_code_and_the_end_of_stderr() {
    let f = Fake::new();
    f.script(
        "pi",
        r#"echo "FIRST-LINE" >&2
i=0; while [ $i -lt 400 ]; do echo "filler line number $i" >&2; i=$((i+1)); done
echo "LAST-LINE" >&2
echo "stdout that is not part of the error"
exit 3
"#,
    );
    let report = f.runner().run(&spec(Harness::Pi), "x", None).await;

    assert!(!report.ok);
    assert_eq!(report.exit_code, Some(3));
    assert_eq!(report.text, "");
    let error = report.error.unwrap();
    assert!(error.starts_with("pi exited with code 3"), "{error}");
    assert!(error.ends_with("LAST-LINE"), "{error}");
    assert!(!error.contains("FIRST-LINE"), "{error}");
    assert!(!error.contains("stdout that is not"), "{error}");
    assert!(error.len() < 2048 + 100, "{}", error.len());
}

#[tokio::test]
async fn unreadable_output_of_a_successful_exit_is_an_error() {
    let f = Fake::new();
    f.script("claude", "cat > /dev/null\necho 'not json at all'\n");
    let report = f.runner().run(&spec(Harness::Claude), "x", None).await;
    assert!(!report.ok);
    assert_eq!(report.exit_code, Some(0));
    assert!(report.error.unwrap().contains("no JSON"));

    f.script(
        "claude",
        "cat > /dev/null\necho '{\"type\":\"result\",\"is_error\":true,\"result\":\"quota\"}'\n",
    );
    let report = f.runner().run(&spec(Harness::Claude), "x", None).await;
    assert!(!report.ok);
    assert!(report.error.unwrap().contains("quota"));
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[tokio::test]
async fn a_timeout_kills_the_harness_and_everything_it_started() {
    let f = Fake::new();
    f.script(
        "pi",
        r#"dir="$(dirname "$0")"
sleep 60 &
echo $! > "$dir/child.pid"
echo "started" >&2
wait
"#,
    );
    let mut s = spec(Harness::Pi);
    s.timeout_secs = 1;
    let started = Instant::now();
    let report = f.runner().run(&s, "x", None).await;

    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(!report.ok);
    assert_eq!(report.exit_code, None);
    let error = report.error.unwrap();
    assert!(error.contains("timed out after 1 s"), "{error}");
    assert!(error.contains("started"), "{error}");
    let pid = fs::read_to_string(f.bin.join("child.pid")).unwrap();
    let pid = pid.trim();
    for _ in 0..50 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the background child {pid} survived the timeout");
}

#[tokio::test]
async fn a_program_that_keeps_the_pipes_open_does_not_hold_the_answer_back() {
    let f = Fake::new();
    f.script("pi", "sleep 6 &\necho 'done'\n");
    let started = Instant::now();
    let report = f.runner().run(&spec(Harness::Pi), "x", None).await;
    assert!(report.ok, "{report:?}");
    assert_eq!(report.text, "done");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_subagent_at_the_depth_limit_cannot_start_another() {
    let f = Fake::new();
    f.script("pi", &format!("{}echo 'ran'\n", recording("pi")));
    let report = f
        .runner()
        .with_depth(1)
        .run(&spec(Harness::Pi), "x", None)
        .await;
    assert!(!report.ok);
    assert!(report.error.unwrap().contains("cannot start subagents"));
    assert!(!f.ran("pi"));

    let report = f
        .runner()
        .with_depth(u32::MAX)
        .run(&spec(Harness::Pi), "x", None)
        .await;
    assert!(!report.ok);
    assert!(!f.ran("pi"));

    let report = f
        .runner()
        .with_depth(0)
        .run(&spec(Harness::Pi), "x", None)
        .await;
    assert!(report.ok);
    assert_eq!(f.recorded("pi", "depth"), b"1");
}

#[tokio::test]
async fn a_long_answer_is_cut_with_a_note() {
    let f = Fake::new();
    f.script(
        "pi",
        "i=0; while [ $i -lt 3000 ]; do echo 'abcdefghijklmnopqrstuvwxyz0123456789'; i=$((i+1)); done\n",
    );
    let report = f.runner().run(&spec(Harness::Pi), "x", None).await;
    assert!(report.ok);
    assert!(
        report.text.contains("[output cut at 50 KB;"),
        "tail missing"
    );
    let body = report.text.split("\n\n[output cut").next().unwrap();
    assert_eq!(body.len(), MAX_ANSWER_BYTES);
}

#[tokio::test]
async fn at_most_four_runs_go_at_once() {
    let f = Fake::new();
    f.script(
        "pi",
        r#"dir="$(dirname "$0")"
echo start >> "$dir/log"
sleep 0.4
echo end >> "$dir/log"
echo ok
"#,
    );
    let runner = Arc::new(f.runner());
    let s = spec(Harness::Pi);
    let runs: Vec<_> = (0..(MAX_CONCURRENT + 3))
        .map(|_| {
            let (runner, s) = (runner.clone(), s.clone());
            tokio::spawn(async move { runner.run(&s, "x", None).await })
        })
        .collect();
    for run in runs {
        assert!(run.await.unwrap().ok);
    }
    let log = fs::read_to_string(f.bin.join("log")).unwrap();
    let (mut now, mut peak, mut starts) = (0i32, 0i32, 0);
    for line in log.lines() {
        if line == "start" {
            now += 1;
            starts += 1;
        } else {
            now -= 1;
        }
        peak = peak.max(now);
    }
    assert_eq!(starts, MAX_CONCURRENT + 3);
    assert_eq!(peak, MAX_CONCURRENT as i32, "{log}");
}

#[tokio::test]
async fn a_missing_binary_and_a_bad_cwd_are_reported_without_running_anything() {
    let f = Fake::new();
    let report = f.runner().run(&spec(Harness::Codex), "x", None).await;
    assert!(!report.ok);
    assert_eq!(report.error.unwrap(), "no codex executable found");

    f.script("pi", &format!("{}echo ok\n", recording("pi")));
    for cwd in ["relative/dir", "/nonexistent/nativemem-test-dir"] {
        let report = f
            .runner()
            .run(&spec(Harness::Pi), "x", Some(Path::new(cwd)))
            .await;
        assert!(!report.ok, "{cwd}");
        assert!(report.error.unwrap().contains("cwd"));
    }
    assert!(!f.ran("pi"));
}

// The hub end to end, over an in-memory transport and an rmcp client.

fn definitions() -> SubagentConfig {
    let mut config = SubagentConfig::default();
    let mut reviewer = SubagentSpec::new(Harness::Pi, "pi-model", "Reviews a diff");
    reviewer.timeout_secs = 20;
    config.agents.insert("code-review".into(), reviewer);
    let mut broken = SubagentSpec::new(Harness::Claude, "sonnet", "Always fails");
    broken.timeout_secs = 20;
    config.agents.insert("broken".into(), broken);
    config
}

async fn hub(runner: Runner, state: &Path) -> RunningService<RoleClient, ClientConfig> {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::new(state));
    let state = state.to_path_buf();
    tokio::spawn(async move {
        let server = HubServer::new(HubConfig::default(), store, state)
            .with_subagents(definitions(), runner);
        if let Ok(server) = server.serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(client_io)
    .await
    .expect("the hub initialises")
}

async fn call(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &str,
    arguments: Value,
) -> Result<(String, bool), rmcp::ServiceError> {
    let result = client
        .peer()
        .call_tool(
            CallToolRequestParams::new(tool.to_string())
                .with_arguments(arguments.as_object().cloned().unwrap()),
        )
        .await?;
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, result.is_error == Some(true)))
}

#[tokio::test]
async fn the_hub_lists_and_calls_subagent_tools() {
    let f = Fake::new();
    f.script("pi", &format!("{}echo \"pi saw: $5\"\n", recording("pi")));
    f.script("claude", "cat > /dev/null\necho 'boom' >&2\nexit 7\n");
    let state = tempfile::tempdir().unwrap();
    let client = hub(f.runner(), state.path()).await;

    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "nativemem_status",
            "subagents_list",
            "subagent_broken",
            "subagent_code-review"
        ]
    );
    let review = tools
        .iter()
        .find(|t| t.name == "subagent_code-review")
        .unwrap();
    assert_eq!(
        review.description.as_deref(),
        Some("Reviews a diff (harness pi, model pi-model)")
    );
    let schema = serde_json::to_value(&review.input_schema).unwrap();
    assert_eq!(schema["required"], json!(["prompt"]));
    assert_eq!(schema["properties"]["prompt"]["type"], "string");
    assert_eq!(schema["properties"]["cwd"]["type"], "string");

    let (list, is_error) = call(&client, "subagents_list", json!({})).await.unwrap();
    assert!(!is_error);
    let list: Value = serde_json::from_str(&list).unwrap();
    assert_eq!(
        list["subagents"][1],
        json!({
            "name": "code-review",
            "tool": "subagent_code-review",
            "harness": "pi",
            "model": "pi-model",
            "description": "Reviews a diff"
        })
    );

    let cwd = f.work.display().to_string();
    let (text, is_error) = call(
        &client,
        "subagent_code-review",
        json!({ "prompt": "look at this", "cwd": cwd }),
    )
    .await
    .unwrap();
    assert!(!is_error, "{text}");
    assert_eq!(text, "pi saw: look at this");
    assert_eq!(
        fs::read_to_string(f.bin.join("pi.cwd")).unwrap().trim(),
        cwd
    );

    let (text, is_error) = call(&client, "subagent_broken", json!({ "prompt": "x" }))
        .await
        .unwrap();
    assert!(is_error);
    assert!(text.starts_with("claude exited with code 7"), "{text}");
    assert!(text.ends_with("boom"), "{text}");

    assert!(call(&client, "subagent_code-review", json!({}))
        .await
        .is_err());
    assert!(call(
        &client,
        "subagent_code-review",
        json!({ "prompt": "x", "cwd": 3 })
    )
    .await
    .is_err());
    assert!(call(&client, "subagent_missing", json!({ "prompt": "x" }))
        .await
        .is_err());
}

#[tokio::test]
async fn a_hub_started_by_a_subagent_refuses_to_run_one() {
    let f = Fake::new();
    f.script("pi", &format!("{}echo ok\n", recording("pi")));
    let state = tempfile::tempdir().unwrap();
    let client = hub(f.runner().with_depth(1), state.path()).await;

    // The tools are still listed; calling one is refused.
    let tools = client.peer().list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "subagent_code-review"));
    let (text, is_error) = call(&client, "subagent_code-review", json!({ "prompt": "x" }))
        .await
        .unwrap();
    assert!(is_error);
    assert!(text.contains("cannot start subagents"), "{text}");
    assert!(!f.ran("pi"));
}

#[tokio::test]
async fn a_hub_without_definitions_lists_no_subagents() {
    let state = tempfile::tempdir().unwrap();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::new(state.path()));
    let path = state.path().to_path_buf();
    tokio::spawn(async move {
        if let Ok(server) = HubServer::new(HubConfig::default(), store, path)
            .serve(server_io)
            .await
        {
            let _ = server.waiting().await;
        }
    });
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(client_io)
    .await
    .unwrap();
    let (list, _) = call(&client, "subagents_list", json!({})).await.unwrap();
    assert_eq!(list, r#"{"subagents":[]}"#);
}
