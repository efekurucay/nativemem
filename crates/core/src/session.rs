//! Neutral chat session model and the pure helpers every session adapter shares: canonical
//! serialisation, content hash, tool flattening, timestamps and derived ids.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::model::{AgentId, Item, ItemKey, Kind, Observed, Scope};

/// Sources larger than this are skipped and reported.
pub const MAX_SOURCE_BYTES: u64 = 200 * 1024 * 1024;
/// Tool output shown in a turn is cut to this many characters.
pub const TOOL_RESULT_LIMIT: usize = 1500;
/// Tool input shown in a turn is cut to this many characters.
pub const TOOL_INPUT_LIMIT: usize = 600;
const TITLE_LIMIT: usize = 200;
const UNTITLED: &str = "Untitled session";
const IMAGE_PLACEHOLDER: &str = "[external unsupported block: image]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn parse(s: &str) -> Option<Role> {
        match s {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    pub ts_ms: i64,
}

impl Turn {
    pub fn new(role: Role, text: impl Into<String>, ts_ms: i64) -> Turn {
        Turn {
            role,
            text: text.into(),
            ts_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub turns: Vec<Turn>,
}

pub fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Session {
    pub fn has_user_turn(&self) -> bool {
        self.turns.iter().any(|t| t.role == Role::User)
    }

    /// Deterministic JSON: the `Item` body that travels through the engine.
    pub fn to_body(&self) -> String {
        let turns: Vec<Value> = self
            .turns
            .iter()
            .map(|t| json!({ "role": t.role.as_str(), "text": t.text, "ts_ms": t.ts_ms }))
            .collect();
        json!({
            "id": self.id,
            "cwd": self.cwd.to_string_lossy(),
            "title": self.title,
            "created_ms": self.created_ms,
            "updated_ms": self.updated_ms,
            "turns": turns,
        })
        .to_string()
    }

    pub fn from_body(body: &str) -> Option<Session> {
        let v: Value = serde_json::from_str(body).ok()?;
        let turns = v
            .get("turns")?
            .as_array()?
            .iter()
            .map(|t| {
                Some(Turn {
                    role: Role::parse(t.get("role")?.as_str()?)?,
                    text: t.get("text")?.as_str()?.to_string(),
                    ts_ms: t.get("ts_ms")?.as_i64()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Session {
            id: v.get("id")?.as_str()?.to_string(),
            cwd: PathBuf::from(v.get("cwd")?.as_str()?),
            title: v.get("title")?.as_str()?.to_string(),
            created_ms: v.get("created_ms")?.as_i64()?,
            updated_ms: v.get("updated_ms")?.as_i64()?,
            turns,
        })
    }

    /// SHA-256 over the working directory, title, and each turn's role and text. Ids and
    /// timestamps are left out: they differ between an original and its replica.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        let mut field = |s: &str| {
            h.update((s.len() as u64).to_le_bytes());
            h.update(s.as_bytes());
        };
        field(&self.cwd.to_string_lossy());
        field(&self.title);
        for t in &self.turns {
            field(t.role.as_str());
            field(&t.text);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The engine item: kind `Session`, scope `Project(cwd)`, `name` the session's own id,
    /// `description` its title, `body` the canonical serialisation.
    pub fn to_item(&self) -> Item {
        Item {
            kind: Kind::Session,
            scope: Scope::Project(self.cwd.clone()),
            name: self.id.clone(),
            description: self.title.clone(),
            category: None,
            body: self.to_body(),
            source_dir: None,
        }
    }

    pub fn observed(
        &self,
        agent: &AgentId,
        native_id: String,
        path: PathBuf,
        replica_of: Option<ItemKey>,
    ) -> Observed {
        Observed {
            agent: agent.clone(),
            native_id,
            path,
            item: self.to_item(),
            replica_of,
        }
    }
}

/// Session native ids end in `.jsonl`; memory ids end in `.md`, instruction ids contain `#`.
pub fn is_session_id(native_id: &str) -> bool {
    native_id.ends_with(".jsonl")
}

/// What an adapter found in its session store.
#[derive(Debug, Default)]
pub struct SessionScan {
    pub observed: Vec<Observed>,
    /// `(label, reason)` for sessions that cannot be synced.
    pub skipped: Vec<(String, String)>,
    /// Automated sessions found, whether or not they are left out of `observed`.
    pub automated: usize,
}

/// What `status` reports about an agent's session store besides the sessions themselves.
#[derive(Debug, Default)]
pub struct SessionReport {
    pub skipped: Vec<(String, String)>,
    pub automated: usize,
}

impl SessionScan {
    pub fn report(self) -> SessionReport {
        SessionReport {
            skipped: self.skipped,
            automated: self.automated,
        }
    }
}

/// `<origin agent>:<origin native id>`, the text every replica marker carries.
pub fn marker_text(key: &ItemKey) -> String {
    format!("{}:{}", key.origin, key.native_id)
}

pub fn parse_marker(text: &str) -> Option<ItemKey> {
    let (agent, id) = text.split_once(':')?;
    (!agent.is_empty() && !id.is_empty()).then(|| ItemKey {
        origin: AgentId::new(agent),
        native_id: id.to_string(),
    })
}

/// A single-line title of bounded length; never empty. Idempotent.
pub fn clean_title(raw: &str) -> String {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = one_line.chars().take(TITLE_LIMIT).collect();
    let cut = cut.trim_end().to_string();
    if cut.is_empty() {
        UNTITLED.to_string()
    } else {
        cut
    }
}

/// The title when no agent-provided one exists: the first user turn.
pub fn title_from_turns(turns: &[Turn]) -> String {
    turns
        .iter()
        .find(|t| t.role == Role::User)
        .map_or_else(|| UNTITLED.to_string(), |t| clean_title(&t.text))
}

/// First `max - 3` characters plus `...` when `s` is longer than `max` characters.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max - 3).collect();
    format!("{head}...")
}

/// Tool activity is shown as one assistant text turn per call, call and result together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// Non-zero exit code.
    Exit(i64),
    /// The tool reported an error without an exit code.
    Error,
}

/// Backtick fence long enough that `text` cannot close it.
fn fenced(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{text}\n{fence}")
}

/// `text` cut to `max` characters, with a one-line notice when something was cut.
fn cut_block(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return fenced(text);
    }
    let head: String = text.chars().take(max).collect();
    format!("{}\n… ({} more characters)", fenced(&head), total - max)
}

/// The merged text of one tool call and, when known, its result.
pub fn render_tool(name: &str, input: &str, result: Option<(&str, Outcome)>) -> String {
    let mut out = format!("Tool: {name}");
    if !input.is_empty() {
        out.push('\n');
        out.push_str(&cut_block(input, TOOL_INPUT_LIMIT));
    }
    if let Some((text, outcome)) = result {
        let label = match outcome {
            Outcome::Ok => "Result:".to_string(),
            Outcome::Exit(code) => format!("Result (exit code {code}):"),
            Outcome::Error => "Result (error):".to_string(),
        };
        out.push('\n');
        out.push_str(&label);
        if text.is_empty() {
            out.push_str(" (no output)");
        } else {
            out.push('\n');
            out.push_str(&cut_block(text, TOOL_RESULT_LIMIT));
        }
    }
    out
}

/// A JS string literal starting at the opening quote of `s`: its decoded value.
fn js_string(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let quote = chars.next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == quote {
            return Some(out);
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                out.push(
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)?,
                );
            }
            other => out.push(other),
        }
    }
    None
}

/// The `cmd` string of a JS-style call such as `text(await tools.exec_command({cmd:"ls"}))`.
fn js_cmd(code: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = code[from..].find("cmd") {
        let start = from + at;
        from = start + 3;
        let rest = code[from..].trim_start_matches(['"', '\'']).trim_start();
        if let Some(value) = rest.strip_prefix(':') {
            if let Some(cmd) = js_string(value.trim_start()) {
                return Some(cmd);
            }
        }
    }
    None
}

/// The essential input of a tool call: a shell command, or compact JSON.
pub fn tool_input_text(input: &Value) -> String {
    match input {
        Value::String(raw) => js_cmd(raw).unwrap_or_else(|| raw.trim().to_string()),
        Value::Null => String::new(),
        Value::Object(map) => {
            for key in ["command", "cmd"] {
                match map.get(key) {
                    Some(Value::String(c)) if !c.is_empty() => return c.clone(),
                    Some(Value::Array(parts)) => {
                        let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                        if words.len() >= 3 && matches!(words[1], "-c" | "-lc") {
                            return words[2..].join(" ");
                        }
                        if !words.is_empty() {
                            return words.join(" ");
                        }
                    }
                    _ => {}
                }
            }
            serde_json::to_string(input).unwrap_or_default()
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// A tool output with Codex's JSON wrapper removed: the `output` field and a non-zero exit code.
pub fn unwrap_output(raw: &str) -> (String, Option<i64>) {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw.trim()) {
        if let Some(Value::String(output)) = map.get("output") {
            let code = map
                .get("exit_code")
                .or_else(|| map.get("metadata").and_then(|m| m.get("exit_code")))
                .and_then(Value::as_i64)
                .filter(|c| *c != 0);
            return (output.trim().to_string(), code);
        }
    }
    (raw.trim().to_string(), None)
}

struct Pending {
    id: Option<String>,
    name: String,
    input: String,
    index: usize,
}

/// Collects turns and merges each tool result into the turn of its call.
#[derive(Default)]
pub struct TurnBuilder {
    pub turns: Vec<Turn>,
    pending: Vec<Pending>,
}

impl TurnBuilder {
    /// A plain text turn; empty text adds nothing.
    pub fn text(&mut self, role: Role, text: impl Into<String>, ts_ms: i64) {
        let text = text.into();
        if !text.is_empty() {
            self.turns.push(Turn::new(role, text, ts_ms));
        }
    }

    pub fn call(&mut self, id: Option<&str>, name: &str, input: &Value, ts_ms: i64) {
        let input = tool_input_text(input);
        self.pending.push(Pending {
            id: id.map(str::to_string),
            name: name.to_string(),
            index: self.turns.len(),
            input: input.clone(),
        });
        self.turns.push(Turn::new(
            Role::Assistant,
            render_tool(name, &input, None),
            ts_ms,
        ));
    }

    /// The result of the oldest open call with `id` (calls without an id pair among themselves). Without a
    /// matching call it becomes a turn of its own.
    pub fn result(&mut self, id: Option<&str>, raw: &str, is_error: bool, ts_ms: i64) {
        let (text, code) = unwrap_output(raw);
        let outcome = match (code, is_error) {
            (Some(c), _) => Outcome::Exit(c),
            (None, true) => Outcome::Error,
            (None, false) => Outcome::Ok,
        };
        let at = self.pending.iter().position(|p| p.id.as_deref() == id);
        match at {
            Some(i) => {
                let p = self.pending.remove(i);
                self.turns[p.index].text = render_tool(&p.name, &p.input, Some((&text, outcome)));
            }
            None => self.turns.push(Turn::new(
                Role::Assistant,
                render_tool("result", "", Some((&text, outcome))),
                ts_ms,
            )),
        }
    }
}

/// Text of a content-block array the way a user turn is read: text blocks joined by a blank
/// line, images as a placeholder, every other block ignored.
pub fn join_user_blocks(blocks: &[Value]) -> String {
    let parts: Vec<&str> = blocks
        .iter()
        .filter_map(|b| match b.get("type").and_then(Value::as_str) {
            Some("text") => b.get("text").and_then(Value::as_str),
            Some("image") => Some(IMAGE_PLACEHOLDER),
            _ => None,
        })
        .collect();
    parts.join("\n\n")
}

/// Tool result content: a string, or an array whose text parts are joined by a newline.
pub fn result_content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                (p.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| p.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DDTHH:MM:SS(.fff)?Z` to epoch milliseconds.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    let ms: i64 = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        format!("{digits:0<3}").parse().ok()?
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(((days_from_civil(y, m, day) * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms)
}

/// Epoch milliseconds to `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_iso_ms(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem % 3_600_000 / 60_000,
        rem % 60_000 / 1000,
        rem % 1000
    )
}

/// A version-4-shaped UUID derived from `seed`, so a replica keeps the same ids across refreshes.
pub fn derived_uuid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A UUIDv7: `ms` (epoch milliseconds) in the leading 48 bits, the rest derived from `seed`, so
/// the id is stable for one replica and sorts by creation time.
pub fn uuid_v7(ms: i64, seed: &str) -> String {
    let d = Sha256::digest(seed.as_bytes());
    let mut b = [0u8; 16];
    b[..6].copy_from_slice(&(ms.max(0) as u64).to_be_bytes()[2..]);
    b[6] = 0x70 | (d[0] & 0x0f);
    b[7] = d[1];
    b[8] = 0x80 | (d[2] & 0x3f);
    b[9..].copy_from_slice(&d[3..10]);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn system_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// How sessions are read. Unset limits mean every session of every age and kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionOptions {
    /// Only sessions whose file was modified in the last N days.
    pub days: Option<u32>,
    /// Leave headless runs and sub-agent threads out.
    pub exclude_automated: bool,
}

impl SessionOptions {
    pub fn window(&self) -> SessionWindow {
        SessionWindow { days: self.days }
    }
}

/// Sessions are read only when their file was modified inside the window, if there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionWindow {
    pub days: Option<u32>,
}

/// Outcome of the cheap checks done before a source file is parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Outside,
    TooLarge(u64),
    Read { mtime_ms: i64 },
}

impl SessionWindow {
    pub fn check(&self, path: &Path) -> Result<Source> {
        let meta = fs::metadata(path).map_err(|e| Error::io(path, e))?;
        let mtime = meta.modified().map_err(|e| Error::io(path, e))?;
        if let Some(days) = self.days {
            let cutoff = SystemTime::now()
                .checked_sub(Duration::from_secs(u64::from(days) * 86_400))
                .unwrap_or(UNIX_EPOCH);
            if mtime < cutoff {
                return Ok(Source::Outside);
            }
        }
        if meta.len() > MAX_SOURCE_BYTES {
            return Ok(Source::TooLarge(meta.len()));
        }
        Ok(Source::Read {
            mtime_ms: system_ms(mtime),
        })
    }
}

pub fn too_large_reason(bytes: u64) -> String {
    format!(
        "larger than {} MB ({} MB)",
        MAX_SOURCE_BYTES / (1024 * 1024),
        bytes / (1024 * 1024)
    )
}

/// Sets a file's modification time to `ms`; a failure leaves the time the write gave it.
pub fn set_mtime(path: &Path, ms: i64) {
    let time = if ms >= 0 {
        UNIX_EPOCH + Duration::from_millis(ms as u64)
    } else {
        UNIX_EPOCH
    };
    if let Ok(f) = fs::File::options().write(true).open(path) {
        let _ = f.set_modified(time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: "s1".into(),
            cwd: "/w/app".into(),
            title: "Fix login".into(),
            created_ms: 1_000,
            updated_ms: 2_000,
            turns: vec![
                Turn::new(Role::User, "hello", 1_000),
                Turn::new(Role::Assistant, "hi\nthere", 2_000),
            ],
        }
    }

    #[test]
    fn body_round_trips_and_is_deterministic() {
        let s = session();
        assert_eq!(s.to_body(), session().to_body());
        assert_eq!(Session::from_body(&s.to_body()), Some(s));
        assert_eq!(Session::from_body("not json"), None);
    }

    #[test]
    fn hash_covers_content_but_not_ids_or_times() {
        let base = session().hash();
        assert_eq!(base.len(), 64);
        let mut other = session();
        other.id = "s2".into();
        other.updated_ms = 9;
        other.turns[0].ts_ms = 5;
        assert_eq!(base, other.hash());
        for change in [
            |s: &mut Session| s.cwd = "/x".into(),
            |s: &mut Session| s.title = "t".into(),
            |s: &mut Session| s.turns[1].text.push('!'),
            |s: &mut Session| s.turns[1].role = Role::User,
            |s: &mut Session| s.turns.push(Turn::new(Role::User, "more", 3)),
        ] {
            let mut s = session();
            change(&mut s);
            assert_ne!(base, s.hash());
        }
    }

    #[test]
    fn session_item_hash_is_the_session_hash() {
        let s = session();
        let mut copy = session();
        copy.id = "replica".into();
        assert_eq!(
            crate::model::content_hash(&s.to_item()),
            crate::model::content_hash(&copy.to_item())
        );
        assert_eq!(s.to_item().name, "s1");
        assert_eq!(s.to_item().description, "Fix login");
    }

    #[test]
    fn merged_tool_turn_reads_like_a_conversation() {
        let mut b = TurnBuilder::default();
        b.call(
            Some("c1"),
            "exec_command",
            &json!("text(await tools.exec_command({cmd:\"cat a\\nb\",max_output_tokens:5000}));"),
            1,
        );
        b.text(Role::Assistant, "thinking aloud", 2);
        b.result(
            Some("c1"),
            "{\"chunk_id\":\"x\",\"exit_code\":2,\"original_token_count\":4,\"output\":\"boom\\n\"}",
            false,
            3,
        );
        assert_eq!(b.turns.len(), 2);
        assert_eq!(
            b.turns[0].text,
            "Tool: exec_command\n```\ncat a\nb\n```\nResult (exit code 2):\n```\nboom\n```"
        );
        assert_eq!(b.turns[0].ts_ms, 1);
        assert!(!b.turns[0].text.contains("external_agent"));
    }

    #[test]
    fn input_forms_reduce_to_the_essential_text() {
        assert_eq!(
            tool_input_text(&json!({"command": "ls -l", "description": "d"})),
            "ls -l"
        );
        assert_eq!(
            tool_input_text(&json!({"command": ["bash", "-lc", "echo hi"]})),
            "echo hi"
        );
        assert_eq!(tool_input_text(&json!({"q": "é"})), "{\"q\":\"é\"}");
        assert_eq!(tool_input_text(&json!("*** patch\n")), "*** patch");
        assert_eq!(
            tool_input_text(&json!("tools.exec({cmd: 'ls \\'x\\''})")),
            "ls 'x'"
        );
        assert_eq!(tool_input_text(&Value::Null), "");
    }

    #[test]
    fn long_input_and_output_are_cut_with_a_notice() {
        let t = render_tool(
            "t",
            &"i".repeat(700),
            Some((&"o".repeat(1600), Outcome::Ok)),
        );
        assert!(t.contains("… (100 more characters)"), "{t}");
        assert_eq!(t.matches("… (").count(), 2);
        assert!(t.contains(&"i".repeat(600)) && !t.contains(&"i".repeat(601)));
        let exact = render_tool("t", "", Some((&"o".repeat(1500), Outcome::Ok)));
        assert!(!exact.contains('…'));
    }

    #[test]
    fn fence_outgrows_backticks_in_the_text() {
        let t = render_tool("t", "echo ```", None);
        assert!(t.contains("````\necho ```\n````"), "{t}");
    }

    #[test]
    fn results_without_a_call_and_calls_without_a_result_still_render() {
        let mut b = TurnBuilder::default();
        b.result(Some("missing"), "plain output", true, 1);
        b.call(Some("c"), "read", &json!({"path": "/a"}), 2);
        assert_eq!(
            b.turns[0].text,
            "Tool: result\nResult (error):\n```\nplain output\n```"
        );
        assert_eq!(b.turns[1].text, "Tool: read\n```\n{\"path\":\"/a\"}\n```");
        assert!(b.turns.iter().all(|t| !t.text.is_empty()));
    }

    #[test]
    fn user_blocks_and_result_content() {
        let blocks = vec![
            json!({"type": "text", "text": "a"}),
            json!({"type": "image", "source": {}}),
            json!({"type": "tool_result", "content": "ignored"}),
            json!({"type": "text", "text": "b"}),
        ];
        assert_eq!(
            join_user_blocks(&blocks),
            "a\n\n[external unsupported block: image]\n\nb"
        );
        let content = json!([
            {"type": "text", "text": "one"},
            {"type": "image"},
            {"type": "text", "text": "two"}
        ]);
        assert_eq!(result_content_text(&content), "one\ntwo");
        assert_eq!(result_content_text(&json!("plain")), "plain");
    }

    #[test]
    fn iso_round_trips() {
        let ms = parse_iso_ms("2026-11-10T12:20:24.802Z").unwrap();
        assert_eq!(format_iso_ms(ms), "2026-11-10T12:20:24.802Z");
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-11-10T12:20:24.5Z"), Some(ms - 302));
        assert_eq!(parse_iso_ms("garbage"), None);
        assert_eq!(format_iso_ms(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn derived_uuid_is_stable_and_shaped() {
        let a = derived_uuid("x");
        assert_eq!(a, derived_uuid("x"));
        assert_ne!(a, derived_uuid("y"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4');
    }

    #[test]
    fn uuid_v7_is_valid_stable_and_sorts_by_time() {
        let a = uuid_v7(1_794_304_800_000, "s");
        assert_eq!(a, uuid_v7(1_794_304_800_000, "s"));
        assert_ne!(a, uuid_v7(1_794_304_800_000, "t"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'7');
        assert!(matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert!(uuid_v7(1_794_304_800_000, "z") < uuid_v7(1_794_304_800_001, "a"));
        assert_eq!(&a[..8], format!("{:08x}", 1_794_304_800_000u64 >> 16));
    }

    #[test]
    fn titles_are_single_line_bounded_and_idempotent() {
        assert_eq!(clean_title("  a\n b  "), "a b");
        assert_eq!(clean_title("   "), UNTITLED);
        let long = "w".repeat(500);
        assert_eq!(clean_title(&long).chars().count(), TITLE_LIMIT);
        assert_eq!(clean_title(&clean_title(&long)), clean_title(&long));
        let turns = vec![Turn::new(Role::User, "first\nline", 0)];
        assert_eq!(title_from_turns(&turns), "first line");
    }

    #[test]
    fn markers_parse() {
        let key = ItemKey {
            origin: AgentId::new("claude"),
            native_id: "-w-app/a.jsonl".into(),
        };
        assert_eq!(parse_marker(&marker_text(&key)), Some(key));
        assert_eq!(parse_marker("nocolon"), None);
    }

    #[test]
    fn window_checks_age_and_size() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.jsonl");
        fs::write(&p, "x").unwrap();
        let w = SessionWindow { days: Some(14) };
        assert!(matches!(w.check(&p).unwrap(), Source::Read { .. }));
        set_mtime(&p, system_ms(SystemTime::now()) - 15 * 86_400_000);
        assert_eq!(w.check(&p).unwrap(), Source::Outside);
        set_mtime(&p, system_ms(SystemTime::now()) - 13 * 86_400_000);
        assert!(matches!(w.check(&p).unwrap(), Source::Read { .. }));
        set_mtime(&p, system_ms(SystemTime::now()) - 900 * 86_400_000);
        assert!(matches!(
            SessionWindow { days: None }.check(&p).unwrap(),
            Source::Read { .. }
        ));
    }
}
