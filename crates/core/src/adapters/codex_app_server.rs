//! The native app-server append API operates on the existing rollout.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::session::{derived_uuid, Role, Session};

/// An allocated UUIDv8 prefix distinguishes appended messages from Codex conversation events.
pub(super) const APPENDED_MESSAGE_ID: &str = "msg_1d961050-4d1b-8";
const TIMEOUT: Duration = Duration::from_secs(10);

fn unavailable(operation: &str) -> Error {
    Error::unsupported("codex", operation)
}

pub(super) fn items(current: &Session, head: &Session) -> Result<Vec<Value>> {
    let tail = head
        .appended_turns(current)
        .ok_or_else(|| unavailable("append requires a compatible continuation"))?;
    let base = current.hash();
    tail.iter()
        .enumerate()
        .map(|(index, turn)| {
            let input = match turn.role {
                Role::User => "input_text",
                Role::Assistant => "output_text",
                Role::Summary => return Err(unavailable("append of a compaction summary")),
            };
            let uuid = derived_uuid(&format!("{}:{base}:{index}", current.id));
            Ok(json!({
                "type": "message",
                "id": format!("{APPENDED_MESSAGE_ID}{}{}", if index == 0 { '0' } else { '1' }, &uuid[16..]),
                "role": turn.role.as_str(),
                "content": [{ "type": input, "text": turn.text }],
            }))
        })
        .collect()
}

pub(super) struct AppServer {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<Value>,
    next_id: u32,
}

impl AppServer {
    pub(super) fn start(exe: &Path, home: &Path, cwd: &Path) -> Result<Self> {
        let mut child = Command::new(exe)
            .args(["-c", "check_for_update_on_startup=false"])
            .arg("app-server")
            .current_dir(cwd)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable("start native app-server"))?;
        let input = child.stdin.take();
        let output = child.stdout.take().expect("app-server stdout is piped");
        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if value.get("id").is_some() && sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let mut server = Self {
            child,
            input,
            responses,
            next_id: 0,
        };
        server.call(
            "initialize",
            json!({
                "clientInfo": { "name": "session-sync", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true },
            }),
        )?;
        server.send(json!({ "method": "initialized" }))?;
        Ok(server)
    }

    fn send(&mut self, message: Value) -> Result<()> {
        let input = self.input.as_mut().expect("app-server stdin is piped");
        serde_json::to_writer(&mut *input, &message)
            .map_err(|_| unavailable("send a request to native app-server"))?;
        input
            .write_all(b"\n")
            .map_err(|_| unavailable("send a request to native app-server"))
    }

    pub(super) fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        self.send(json!({ "id": self.next_id, "method": method, "params": params }))?;
        let deadline = Instant::now() + TIMEOUT;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let Ok(response) = self.responses.recv_timeout(remaining) else {
                break;
            };
            if response["id"].as_u64() == Some(u64::from(self.next_id)) {
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| unavailable(&format!("native {method} was rejected")));
            }
        }
        Err(unavailable(&format!("native {method} did not respond")))
    }

    pub(super) fn resume(&mut self, thread: &str, path: &Path) -> Result<()> {
        let result = self.call(
            "thread/resume",
            json!({ "threadId": thread, "excludeTurns": true }),
        )?;
        let info = &result["thread"];
        let resolved = info["path"]
            .as_str()
            .and_then(|p| Path::new(p).canonicalize().ok());
        if info["id"].as_str() != Some(thread) || resolved.as_deref() != Some(path) {
            return Err(unavailable("native resume resolved a different rollout"));
        }
        if info["status"]["type"] != "idle" {
            return Err(unavailable("the native thread is not idle"));
        }
        Ok(())
    }

    pub(super) fn append(mut self, thread: &str, items: Vec<Value>) -> Result<()> {
        self.call(
            "thread/inject_items",
            json!({ "threadId": thread, "items": items }),
        )?;
        self.close()
    }

    pub(super) fn close(mut self) -> Result<()> {
        self.input.take();
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                _ => break,
            }
        }
        Err(unavailable("the native app-server did not finish cleanly"))
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
