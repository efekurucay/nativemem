//! Runs one subagent: the harness process headless in a working directory, with a timeout, at most
//! `MAX_CONCURRENT` at a time per process, and a depth guard so a subagent cannot start subagents.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::error::{HubError, Result};
use crate::harness::{self, Invocation};
use crate::subagents::SubagentSpec;

/// The nesting level of the current process: a subagent's process sees its parent's level plus one.
pub const DEPTH_VAR: &str = "NATIVEMEM_SUBAGENT_DEPTH";

/// A run whose level would reach this is refused.
pub const MAX_DEPTH: u32 = 2;

pub const MAX_CONCURRENT: usize = 4;

/// The longest answer returned; the rest is cut with a note.
pub const MAX_ANSWER_BYTES: usize = 50 * 1024;

/// How much of the end of stderr an error carries.
pub const STDERR_TAIL_BYTES: usize = 2048;

/// Output kept from a harness's stdout before the rest is discarded.
const CAPTURE_LIMIT: usize = 32 * 1024 * 1024;

/// After the harness exits, how long a program that kept the pipes open gets to finish writing.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// The outcome of one run. Output and errors never include the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub ok: bool,
    /// The answer, cut at `MAX_ANSWER_BYTES`; empty when the run failed.
    pub text: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

impl RunReport {
    fn failed(error: impl Into<String>, exit_code: Option<i32>, started: Instant) -> Self {
        RunReport {
            ok: false,
            text: String::new(),
            exit_code,
            duration_ms: elapsed_ms(started),
            error: Some(error.into()),
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub struct Runner {
    search_path: OsString,
    depth: u32,
    permits: Arc<Semaphore>,
}

impl Runner {
    /// Looks for the harness binaries in `PATH` and the usual install directories; the nesting level
    /// comes from `NATIVEMEM_SUBAGENT_DEPTH`. A value that is not a number counts as the highest
    /// level, so nothing starts.
    pub fn from_env() -> Runner {
        let depth = match std::env::var(DEPTH_VAR) {
            Err(_) => 0,
            Ok(v) => v.trim().parse().unwrap_or(MAX_DEPTH),
        };
        Runner {
            search_path: harness::search_path(),
            depth,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }

    pub fn with_search_path(mut self, search_path: OsString) -> Self {
        self.search_path = search_path;
        self
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    pub fn search_path(&self) -> &OsString {
        &self.search_path
    }

    /// `prompt` in `cwd` (this process's directory when `None`). Never panics and never returns a
    /// secret: failures are reported in the report.
    pub async fn run(&self, spec: &SubagentSpec, prompt: &str, cwd: Option<&Path>) -> RunReport {
        let started = Instant::now();
        if self.depth.saturating_add(1) >= MAX_DEPTH {
            return RunReport::failed(
                "a subagent cannot start subagents (nesting level limit reached)",
                None,
                started,
            );
        }
        if let Some(cwd) = cwd {
            if !cwd.is_absolute() || !cwd.is_dir() {
                return RunReport::failed(
                    "cwd must be an absolute path of an existing directory",
                    None,
                    started,
                );
            }
        }
        let harness = spec.harness;
        let Some(program) = harness::locate(harness, &self.search_path) else {
            return RunReport::failed(
                format!("no {} executable found", harness.binary()),
                None,
                started,
            );
        };
        let Ok(_permit) = self.permits.clone().acquire_owned().await else {
            return RunReport::failed("the subagent queue is closed", None, started);
        };
        // The wait for a free slot does not count against the run.
        let started = Instant::now();
        let scratch = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(e) => {
                return RunReport::failed(
                    format!("cannot create a scratch directory: {e}"),
                    None,
                    started,
                )
            }
        };
        let invocation = harness::build(&program, spec, prompt, scratch.path());
        self.execute(spec, invocation, cwd, started).await
    }

    async fn execute(
        &self,
        spec: &SubagentSpec,
        invocation: Invocation,
        cwd: Option<&Path>,
        started: Instant,
    ) -> RunReport {
        let harness = spec.harness;
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .env(DEPTH_VAR, self.depth.saturating_add(1).to_string())
            .env("PATH", &self.search_path)
            .stdin(if invocation.stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        // Its own process group, so a timeout can end the harness and everything it started.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return RunReport::failed(
                    format!("cannot start {}: {}", harness.binary(), e.kind()),
                    None,
                    started,
                )
            }
        };
        let mut guard = GroupGuard(child.id());
        if let (Some(mut stdin), Some(prompt)) = (child.stdin.take(), invocation.stdin) {
            // A harness that exits before reading its prompt closes the pipe; the exit code tells.
            tokio::spawn(async move {
                let _ = stdin.write_all(prompt.as_bytes()).await;
            });
        }
        let stdout = Captured::start(child.stdout.take(), Keep::First(CAPTURE_LIMIT));
        let stderr = Captured::start(child.stderr.take(), Keep::Last(STDERR_TAIL_BYTES));

        let limit = Duration::from_secs(spec.timeout_secs);
        let waited = tokio::time::timeout(limit, child.wait()).await;
        let timed_out = waited.is_err();
        let status = match waited {
            Ok(status) => status,
            Err(_) => {
                guard.kill();
                let _ = child.start_kill();
                child.wait().await
            }
        };
        guard.disarm();
        let (stdout, stderr) = (stdout.finish().await, stderr.finish().await);
        let tail = stderr_tail(&stderr);
        let with_stderr = |head: String| {
            if tail.is_empty() {
                head
            } else {
                format!("{head}\nstderr (last 2 KB):\n{tail}")
            }
        };

        if timed_out {
            return RunReport::failed(
                with_stderr(format!(
                    "{} timed out after {} s and was killed",
                    harness.binary(),
                    spec.timeout_secs
                )),
                None,
                started,
            );
        }
        let status = match status {
            Ok(status) => status,
            Err(e) => {
                return RunReport::failed(
                    format!("cannot wait for {}: {}", harness.binary(), e.kind()),
                    None,
                    started,
                )
            }
        };
        let code = status.code();
        if !status.success() {
            let head = match code {
                Some(code) => format!("{} exited with code {code}", harness.binary()),
                None => format!("{} was terminated by a signal", harness.binary()),
            };
            return RunReport::failed(with_stderr(head), code, started);
        }
        let answer_file = match &invocation.answer_file {
            Some(path) => tokio::fs::read_to_string(path).await.ok(),
            None => None,
        };
        match harness::parse_answer(
            harness,
            &String::from_utf8_lossy(&stdout),
            answer_file.as_deref(),
        ) {
            Ok(text) => RunReport {
                ok: true,
                text: cut_answer(&text),
                exit_code: code,
                duration_ms: elapsed_ms(started),
                error: None,
            },
            Err(message) => RunReport::failed(with_stderr(message), code, started),
        }
    }

    /// `run` for callers without an async runtime.
    pub fn run_blocking(
        &self,
        spec: &SubagentSpec,
        prompt: &str,
        cwd: Option<&Path>,
    ) -> Result<RunReport> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HubError::Invalid(format!("cannot start the async runtime: {e}")))?;
        Ok(runtime.block_on(self.run(spec, prompt, cwd)))
    }
}

/// `text` limited to `MAX_ANSWER_BYTES`, with a note when something was cut.
pub fn cut_answer(text: &str) -> String {
    if text.len() <= MAX_ANSWER_BYTES {
        return text.to_string();
    }
    let mut end = MAX_ANSWER_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[output cut at 50 KB; {} bytes omitted]",
        &text[..end],
        text.len() - end
    )
}

/// The last `STDERR_TAIL_BYTES` bytes of stderr as text.
fn stderr_tail(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(STDERR_TAIL_BYTES);
    let text = String::from_utf8_lossy(&bytes[start..]);
    // A cut inside a character leaves a replacement character at the start.
    let text = if start > 0 {
        text.trim_start_matches('\u{FFFD}')
    } else {
        &text
    };
    text.trim().to_string()
}

/// Ends the harness's process group when dropped while armed, so a cancelled run leaves nothing
/// behind.
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn kill(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.and_then(|p| i32::try_from(p).ok()) {
            // SAFETY: killpg takes plain integers and has no memory effects; the group id is the
            // pid of the child this guard owns, which was started as its own group leader.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Clone, Copy)]
enum Keep {
    First(usize),
    Last(usize),
}

/// Output read in the background. What was read so far survives a reader that is still waiting.
struct Captured {
    buffer: Arc<Mutex<Vec<u8>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Captured {
    fn start<R: AsyncRead + Unpin + Send + 'static>(reader: Option<R>, keep: Keep) -> Captured {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let task = reader.map(|mut reader| {
            let buffer = buffer.clone();
            tokio::spawn(async move {
                let mut chunk = [0u8; 8192];
                loop {
                    let n = match reader.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let Ok(mut kept) = buffer.lock() else { break };
                    match keep {
                        Keep::First(limit) => {
                            let room = limit.saturating_sub(kept.len());
                            kept.extend_from_slice(&chunk[..n.min(room)]);
                        }
                        Keep::Last(limit) => {
                            kept.extend_from_slice(&chunk[..n]);
                            if kept.len() > limit * 2 {
                                let cut = kept.len() - limit;
                                kept.drain(..cut);
                            }
                        }
                    }
                }
            })
        });
        Captured { buffer, task }
    }

    /// The output once the pipe closes, or after `PIPE_GRACE` when something else holds it open.
    async fn finish(self) -> Vec<u8> {
        if let Some(mut task) = self.task {
            if tokio::time::timeout(PIPE_GRACE, &mut task).await.is_err() {
                task.abort();
            }
        }
        self.buffer.lock().map(|b| b.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_answer_is_cut_on_a_character_boundary_with_a_note() {
        assert_eq!(cut_answer("short"), "short");
        let exact = "a".repeat(MAX_ANSWER_BYTES);
        assert_eq!(cut_answer(&exact), exact);
        let text = format!("{}é{}", "a".repeat(MAX_ANSWER_BYTES - 1), "b".repeat(10));
        let cut = cut_answer(&text);
        let body = cut.split("\n\n[output cut").next().unwrap();
        assert_eq!(body.len(), MAX_ANSWER_BYTES - 1);
        assert!(
            cut.ends_with("[output cut at 50 KB; 12 bytes omitted]"),
            "{cut}"
        );
    }

    #[test]
    fn the_stderr_tail_keeps_the_end_and_whole_characters() {
        assert_eq!(stderr_tail(b"  oops \n"), "oops");
        let mut bytes = "é".repeat(STDERR_TAIL_BYTES).into_bytes();
        bytes.extend_from_slice(b"end");
        let tail = stderr_tail(&bytes);
        assert!(tail.ends_with("end"));
        assert!(!tail.starts_with('\u{FFFD}'));
        assert!(tail.len() <= STDERR_TAIL_BYTES);
    }
}
