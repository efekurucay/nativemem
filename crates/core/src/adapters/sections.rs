//! Instruction sections of a global instruction file (`CLAUDE.md`, `AGENTS.md`): the section
//! parser, the managed regions and the byte-preserving edits shared by both adapters.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::sha_hex;
use crate::slug::slugify;

pub const BLOCK_BEGIN: &str = "<!-- nativemem:begin -->";
pub const BLOCK_END: &str = "<!-- nativemem:end -->";
const REGION_BEGIN: &str = "<!-- nativemem:instructions:begin -->";
const REGION_END: &str = "<!-- nativemem:instructions:end -->";
const MARKER_PREFIX: &str = "<!-- nativemem origin=";
const MARKER_SUFFIX: &str = " -->";
const HEADING_PREFIX: &str = "## ";

pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The line that marks a replica: `<!-- nativemem origin=<agent> id=<native id> -->`.
pub fn marker(origin: &str, id: &str) -> String {
    format!(
        "{MARKER_PREFIX}{} id={}{MARKER_SUFFIX}",
        encode(origin),
        encode(id)
    )
}

pub fn parse_marker(line: &str) -> Option<(String, String)> {
    let inner = line
        .strip_prefix(MARKER_PREFIX)?
        .strip_suffix(MARKER_SUFFIX)?;
    let (origin, id) = inner.split_once(" id=")?;
    Some((decode(origin)?, decode(id)?))
}

fn span_between(text: &str, begin: &str, end: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = text.find(begin) else {
        return Ok(None);
    };
    let found = text[start..]
        .find(end)
        .ok_or_else(|| Error::Parse(format!("{begin} has no matching {end}")))?;
    Ok(Some((start, start + found + end.len())))
}

/// Byte span of the project-memory routing block, end marker included.
pub fn block_span(text: &str) -> Result<Option<(usize, usize)>> {
    span_between(text, BLOCK_BEGIN, BLOCK_END)
}

/// Byte span of the instruction region, end marker included.
pub fn region_span(text: &str) -> Result<Option<(usize, usize)>> {
    span_between(text, REGION_BEGIN, REGION_END)
}

#[derive(Clone)]
struct Raw {
    heading: String,
    start: usize,
    heading_end: usize,
    end: usize,
}

pub(super) fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = &line[indent..];
    let c = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, &t[n..]))
}

/// Sections found outside the managed `spans`. A section ends at the next heading or where a
/// managed span begins; text after a span belongs to no section until the next heading.
fn raw_sections(text: &str, spans: &[(usize, usize)]) -> Vec<Raw> {
    let mut out = Vec::new();
    let mut open: Option<Raw> = None;
    let mut fence: Option<(char, usize)> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        if let Some(&(s, _)) = spans.iter().find(|&&(s, e)| start < e && pos > s) {
            if let Some(mut r) = open.take() {
                r.end = s.max(r.heading_end);
                out.push(r);
            }
            fence = None;
            continue;
        }
        let content = line.trim_end_matches(['\n', '\r']);
        if let Some((c, n)) = fence {
            if fence_run(content)
                .is_some_and(|(c2, n2, rest)| c2 == c && n2 >= n && rest.trim().is_empty())
            {
                fence = None;
            }
            continue;
        }
        if let Some((c, n, rest)) = fence_run(content) {
            if !(c == '`' && rest.contains('`')) {
                fence = Some((c, n));
            }
            continue;
        }
        let heading = content
            .strip_prefix(HEADING_PREFIX)
            .map(str::trim)
            .filter(|h| !h.is_empty());
        if let Some(h) = heading {
            if let Some(mut r) = open.take() {
                r.end = start;
                out.push(r);
            }
            open = Some(Raw {
                heading: h.to_string(),
                start,
                heading_end: pos,
                end: pos,
            });
        }
    }
    if let Some(mut r) = open {
        r.end = text.len();
        out.push(r);
    }
    out
}

fn managed_spans(text: &str) -> Result<Vec<(usize, usize)>> {
    let mut spans: Vec<_> = block_span(text)?.into_iter().collect();
    spans.extend(region_span(text)?);
    spans.sort();
    Ok(spans)
}

/// Sections outside managed regions with their native ids, in file order. Duplicate headings
/// get `-2`, `-3`, ... after the slug.
#[cfg(test)]
fn located(text: &str, file: &str) -> Result<Vec<(String, Raw)>> {
    Ok(original_ids(
        file,
        raw_sections(text, &managed_spans(text)?),
    ))
}

fn original_ids(file: &str, sections: impl IntoIterator<Item = Raw>) -> Vec<(String, Raw)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in sections {
        let base = format!("{file}#{}", slugify(&raw.heading));
        let mut id = base.clone();
        let mut n = 2;
        while seen.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        seen.insert(id.clone());
        out.push((id, raw));
    }
    out
}

/// Text without leading blank lines and trailing whitespace.
pub fn trim_body(s: &str) -> String {
    let mut skip = 0;
    for line in s.split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        skip += line.len();
    }
    s[skip..].trim_end().to_string()
}

/// Byte range of the section body without its leading blank lines and trailing whitespace.
fn body_range(text: &str, raw: &Raw) -> Option<(usize, usize)> {
    let seg = &text[raw.heading_end..raw.end];
    let mut first = 0;
    let mut found = false;
    for line in seg.split_inclusive('\n') {
        if !line.trim().is_empty() {
            found = true;
            break;
        }
        first += line.len();
    }
    found.then(|| {
        (
            raw.heading_end + first,
            raw.heading_end + seg.trim_end().len(),
        )
    })
}

fn section_body(text: &str, raw: &Raw) -> String {
    body_range(text, raw).map_or_else(String::new, |(s, e)| text[s..e].to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica {
    pub origin: String,
    pub id: String,
    pub heading: String,
    pub body: String,
}

fn parse_region(text: &str) -> Result<Vec<Replica>> {
    let Some((s, e)) = region_span(text)? else {
        return Ok(Vec::new());
    };
    let inner = &text[s + REGION_BEGIN.len()..e - REGION_END.len()];
    let mut chunks: Vec<(String, String, String)> = Vec::new();
    for line in inner.split_inclusive('\n') {
        if let Some((origin, id)) = parse_marker(line.trim_end_matches(['\n', '\r'])) {
            chunks.push((origin, id, String::new()));
        } else if let Some(c) = chunks.last_mut() {
            c.2.push_str(line);
        }
    }
    let mut out = Vec::new();
    for (origin, id, rest) in chunks {
        let (first, body) = rest.split_once('\n').unwrap_or((rest.as_str(), ""));
        let heading = first
            .trim_end_matches('\r')
            .strip_prefix(HEADING_PREFIX)
            .map(str::trim)
            .filter(|h| !h.is_empty());
        if let Some(h) = heading {
            out.push(Replica {
                origin,
                id,
                heading: h.to_string(),
                body: trim_body(body),
            });
        }
    }
    Ok(out)
}

/// Replaces the body of an original section in place; the heading line and every other byte
/// stay. `None` when the section does not exist.
#[cfg(test)]
pub fn rewrite_body(text: &str, file: &str, native_id: &str, body: &str) -> Result<Option<String>> {
    let sections = located(text, file)?;
    let Some((_, raw)) = sections.iter().find(|(id, _)| id == native_id) else {
        return Ok(None);
    };
    Ok(Some(rewrite_section_body(text, raw, body)))
}

fn rewrite_section_body(text: &str, raw: &Raw, body: &str) -> String {
    let body = trim_body(body);
    match body_range(text, raw) {
        Some((s, e)) => format!("{}{body}{}", &text[..s], &text[e..]),
        None if body.is_empty() => text.to_string(),
        None => {
            let head = &text[..raw.heading_end];
            let nl = if head.ends_with('\n') { "" } else { "\n" };
            format!("{head}{nl}\n{body}\n{}", &text[raw.heading_end..])
        }
    }
}

/// Why `heading` and `body` cannot be stored as exactly one new section, or `None`. A heading is
/// one non-empty line; a body must not start another section or hold a managed marker, because a
/// scan would then read back something other than what was written.
pub fn section_problem(heading: &str, body: &str) -> Option<&'static str> {
    if heading.trim().is_empty() || heading.contains(['\n', '\r']) {
        return Some("the heading must be one non-empty line");
    }
    if body.contains("<!-- nativemem") {
        return Some("the body must not contain a nativemem marker");
    }
    let text = render_section(heading.trim(), body);
    let sections = raw_sections(&text, &[]);
    let one = sections.len() == 1
        && sections[0].heading == heading.trim()
        && section_body(&text, &sections[0]) == trim_body(body);
    (!one).then_some("the body must not start another `## ` section")
}

fn render_section(heading: &str, body: &str) -> String {
    let body = trim_body(body);
    if body.is_empty() {
        format!("{HEADING_PREFIX}{heading}\n")
    } else {
        format!("{HEADING_PREFIX}{heading}\n\n{body}\n")
    }
}

/// Adds a new original section before the first managed region (at the end of the file when
/// there is none), separated by one blank line from what precedes and follows it. Every other
/// byte stays. `None` when a section with `native_id` exists.
#[cfg(test)]
pub fn insert_section(
    text: &str,
    file: &str,
    native_id: &str,
    heading: &str,
    body: &str,
) -> Result<Option<String>> {
    if located(text, file)?.iter().any(|(id, _)| id == native_id) {
        return Ok(None);
    }
    let at = managed_spans(text)?.first().map(|&(s, _)| s);
    Ok(Some(insert_at(text, at, heading, body)))
}

fn insert_at(text: &str, at: Option<usize>, heading: &str, body: &str) -> String {
    let section = render_section(heading.trim(), body);
    let before = &text[..at.unwrap_or(text.len())];
    let sep = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    match at {
        Some(s) => format!("{before}{sep}{section}\n{}", &text[s..]),
        None => format!("{before}{sep}{section}"),
    }
}

/// Deletes an original section including the blank lines after it. `None` when the section
/// does not exist.
#[cfg(test)]
pub fn remove_section(text: &str, file: &str, native_id: &str) -> Result<Option<String>> {
    let sections = located(text, file)?;
    let Some((_, raw)) = sections.iter().find(|(id, _)| id == native_id) else {
        return Ok(None);
    };
    Ok(Some(remove_at(text, raw)))
}

fn remove_at(text: &str, raw: &Raw) -> String {
    let (head, tail) = (&text[..raw.start], &text[raw.end..]);
    if tail.is_empty() && !head.trim().is_empty() {
        for ending in ["\r\n", "\n"] {
            if let Some(before) = head.strip_suffix(ending) {
                if before.ends_with(ending) {
                    return before.to_string();
                }
            }
        }
    }
    format!("{head}{tail}")
}

/// How a native id relates to the instruction file `file`.
#[derive(Debug, PartialEq, Eq)]
pub enum IdKind {
    Original,
    Replica { origin: String, id: String },
}

pub fn replica_native_id(file: &str, origin: &str, id: &str) -> String {
    format!("{file}#{origin}:{id}")
}

/// `None` when the id does not address an instruction section of `file`.
pub fn classify(file: &str, native_id: &str) -> Option<IdKind> {
    let rest = native_id.strip_prefix(file)?.strip_prefix('#')?;
    Some(match rest.split_once(':') {
        Some((origin, id)) => IdKind::Replica {
            origin: origin.to_string(),
            id: id.to_string(),
        },
        None => IdKind::Original,
    })
}

fn read_text(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// One agent's global instruction file: scans it and applies item operations to it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Origin {
    agent: String,
    id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Anchor {
    heading: String,
    body: String,
    origin: Option<Origin>,
}

impl Anchor {
    fn new(heading: &str, body: &str, origin: Option<Origin>) -> Self {
        Self {
            heading: sha_hex(heading.as_bytes()),
            body: sha_hex(body.replace("\r\n", "\n").as_bytes()),
            origin,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Snapshot {
    hash: String,
    anchors: Vec<Anchor>,
}

impl Snapshot {
    fn new(text: &str, anchors: Vec<Anchor>) -> Self {
        Self {
            hash: sha_hex(text.as_bytes()),
            anchors,
        }
    }

    fn has_replicas(&self) -> bool {
        self.anchors.iter().any(|anchor| anchor.origin.is_some())
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct InstructionState {
    saved: Snapshot,
    pending: Option<Snapshot>,
}

struct Document {
    current: Option<String>,
    text: String,
    sections: Vec<Raw>,
    anchors: Vec<Anchor>,
    state: Option<InstructionState>,
}

impl Document {
    fn ids(&self, file: &str) -> Vec<String> {
        let originals = self
            .sections
            .iter()
            .zip(&self.anchors)
            .filter(|(_, a)| a.origin.is_none())
            .map(|(raw, _)| raw.clone());
        let mut originals = original_ids(file, originals).into_iter();
        self.anchors
            .iter()
            .map(|a| match &a.origin {
                Some(o) => replica_native_id(file, &o.agent, &o.id),
                None => originals.next().expect("one id per original").0,
            })
            .collect()
    }
}

fn anchors(text: &str, sections: &[Raw]) -> Vec<Anchor> {
    sections
        .iter()
        .map(|raw| Anchor::new(&raw.heading, &section_body(text, raw), None))
        .collect()
}

/// A duplicate can use its relative order when the exact matching group has the same size.
fn match_anchors(saved: &[Anchor], current: &mut [Anchor]) -> Result<()> {
    let mut saved_used = vec![false; saved.len()];
    let mut current_used = vec![false; current.len()];
    let checks: [fn(&Anchor, &Anchor) -> bool; 3] = [
        |a, b| a.heading == b.heading && a.body == b.body,
        |a, b| a.heading == b.heading,
        |a, b| a.body == b.body,
    ];
    for (pass, same) in checks.into_iter().enumerate() {
        for (index, anchor) in saved.iter().enumerate() {
            if saved_used[index] {
                continue;
            }
            let source: Vec<_> = saved
                .iter()
                .enumerate()
                .filter(|(i, a)| !saved_used[*i] && same(anchor, a))
                .map(|(i, _)| i)
                .collect();
            let target: Vec<_> = current
                .iter()
                .enumerate()
                .filter(|(i, a)| !current_used[*i] && same(anchor, a))
                .map(|(i, _)| i)
                .collect();
            if source.len() == target.len() && (pass == 0 || source.len() == 1) {
                for (from, to) in source.into_iter().zip(target) {
                    current[to].origin = saved[from].origin.clone();
                    saved_used[from] = true;
                    current_used[to] = true;
                }
            }
        }
    }
    if current_used.contains(&false)
        && saved
            .iter()
            .zip(saved_used)
            .any(|(a, used)| !used && a.origin.is_some())
    {
        return Err(Error::Parse(
            "cannot identify edited instruction replicas".into(),
        ));
    }
    Ok(())
}

/// Remove only the managed lines and associate each replica with its heading offset.
fn unmark(text: &str) -> Result<(String, Vec<(usize, Origin)>)> {
    let Some((start, end)) = region_span(text)? else {
        return Ok((text.to_string(), Vec::new()));
    };
    let after = &text[end..];
    if (start != 0 && !text[..start].ends_with('\n'))
        || !(after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n"))
    {
        return Err(Error::Parse(
            "instruction region markers must occupy complete lines".into(),
        ));
    }
    let after = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    let mut plain = text[..start].to_string();
    let mut origins = Vec::new();
    let mut lines = text[start..end].split_inclusive('\n');
    if lines.next().map(|line| line.trim_end_matches(['\r', '\n'])) != Some(REGION_BEGIN) {
        return Err(Error::Parse("invalid instruction region beginning".into()));
    }
    for line in lines {
        let content = line.trim_end_matches(['\r', '\n']);
        if content == REGION_END {
            continue;
        }
        if let Some((agent, id)) = parse_marker(content) {
            origins.push((plain.len(), Origin { agent, id }));
        } else {
            plain.push_str(line);
        }
    }
    plain.push_str(after);
    Ok((plain, origins))
}

pub struct InstructionFile {
    agent: AgentId,
    root: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    file: &'static str,
}

impl InstructionFile {
    pub fn new(
        agent: AgentId,
        root: PathBuf,
        state_dir: PathBuf,
        backups: BackupSession,
        file: &'static str,
    ) -> Self {
        InstructionFile {
            agent,
            root,
            state_dir,
            backups,
            file,
        }
    }

    pub fn owns(&self, native_id: &str) -> bool {
        classify(self.file, native_id).is_some()
    }

    fn path(&self) -> PathBuf {
        self.root.join(self.file)
    }

    fn state_path(&self) -> PathBuf {
        let root = self
            .root
            .ancestors()
            .find_map(|ancestor| {
                let suffix = self.root.strip_prefix(ancestor).ok()?;
                let parent = if ancestor.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    ancestor
                };
                Some(parent.canonicalize().ok()?.join(suffix))
            })
            .unwrap_or_else(|| self.root.clone());
        let profile = sha_hex(root.as_os_str().as_encoded_bytes());
        self.state_dir
            .join("instruction-origins")
            .join(self.agent.as_str())
            .join(format!("{profile}.json"))
    }

    fn load(&self) -> Result<Document> {
        let current = read_text(&self.path())?;
        let native = current.as_deref().unwrap_or_default();
        let state_path = self.state_path();
        let state: Option<InstructionState> = read_text(&state_path)?
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| Error::Parse(format!("{}: {e}", state_path.display())))
            })
            .transpose()?;
        let (text, inline) = unmark(native)?;
        let sections = raw_sections(&text, &managed_spans(&text)?);
        let mut slots = anchors(&text, &sections);
        let hash = sha_hex(native.as_bytes());
        let saved = if let Some(state) = &state {
            if let Some(pending) = &state.pending {
                if pending.hash == hash {
                    Some(pending)
                } else if state.saved.hash == hash {
                    Some(&state.saved)
                } else {
                    return Err(Error::Parse(format!(
                        "{} changed during an interrupted instruction write",
                        self.path().display()
                    )));
                }
            } else {
                Some(&state.saved)
            }
        } else {
            None
        };
        if text != native {
            for (offset, origin) in inline {
                let index = sections
                    .iter()
                    .position(|raw| raw.start == offset)
                    .ok_or_else(|| self.invalid_layout())?;
                slots[index].origin = Some(origin);
            }
            let plain: Vec<_> = slots
                .iter()
                .filter(|a| a.origin.is_none())
                .cloned()
                .collect();
            let before = raw_sections(native, &managed_spans(native)?);
            let replicas: Vec<_> = slots
                .iter()
                .filter(|a| a.origin.is_some())
                .cloned()
                .collect();
            let expected: Vec<_> = parse_region(native)?
                .into_iter()
                .map(|r| {
                    Anchor::new(
                        &r.heading,
                        &r.body,
                        Some(Origin {
                            agent: r.origin,
                            id: r.id,
                        }),
                    )
                })
                .collect();
            if plain != anchors(native, &before)
                || replicas != expected
                || saved.is_some_and(|s| s.has_replicas() && s.anchors != slots)
            {
                return Err(self.invalid_layout());
            }
        } else if let Some(saved) = saved.filter(|s| s.has_replicas()) {
            if saved.hash == hash {
                if saved.anchors.len() != slots.len() {
                    return Err(self.invalid_layout());
                }
                slots = saved.anchors.clone();
            } else {
                match_anchors(&saved.anchors, &mut slots)?;
            }
        }
        Ok(Document {
            current,
            text,
            sections,
            anchors: slots,
            state,
        })
    }

    fn invalid_layout(&self) -> Error {
        Error::Parse(format!(
            "cannot preserve instruction sections in {}",
            self.path().display()
        ))
    }

    fn item(heading: String, body: String) -> Item {
        Item {
            kind: Kind::Instruction,
            scope: Scope::Global,
            name: slugify(&heading),
            description: heading,
            category: None,
            body,
            source_dir: None,
        }
    }

    pub fn scan(&self) -> Result<Vec<Observed>> {
        let document = self.load()?;
        Ok(document
            .ids(self.file)
            .into_iter()
            .zip(&document.sections)
            .zip(&document.anchors)
            .map(|((native_id, raw), anchor)| Observed {
                agent: self.agent.clone(),
                native_id,
                path: self.path(),
                item: Self::item(raw.heading.clone(), section_body(&document.text, raw)),
                replica_of: anchor.origin.as_ref().map(|origin| ItemKey {
                    origin: AgentId::new(&origin.agent),
                    native_id: origin.id.clone(),
                }),
            })
            .collect())
    }

    fn require_global(&self, item: &Item, operation: &str) -> Result<()> {
        if item.scope == Scope::Global {
            Ok(())
        } else {
            Err(Error::unsupported(
                self.agent.as_str(),
                format!("{operation} for non-global instruction"),
            ))
        }
    }

    fn save_state(&self, state: &InstructionState) -> Result<()> {
        let path = self.state_path();
        if path.exists() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &self.agent,
                &self.state_dir,
                &path,
            )?;
        }
        let bytes = serde_json::to_vec(state).map_err(|e| Error::Parse(e.to_string()))?;
        fsx::atomic_write(&path, &bytes)
    }

    fn store(&self, document: Document, updated: &str, expected: Vec<Anchor>) -> Result<()> {
        let found = anchors(updated, &raw_sections(updated, &managed_spans(updated)?));
        if found.len() != expected.len()
            || found
                .iter()
                .zip(&expected)
                .any(|(a, b)| a.heading != b.heading || a.body != b.body)
        {
            return Err(self.invalid_layout());
        }
        let path = self.path();
        let current = document.current.as_deref();
        let changed = current.unwrap_or_default() != updated;
        let old = Snapshot::new(current.unwrap_or_default(), document.anchors);
        let next = Snapshot::new(updated, expected);
        let track = next.has_replicas() || document.state.is_some();
        let done = InstructionState {
            saved: next,
            pending: None,
        };
        if !changed && (!track || document.state.as_ref() == Some(&done)) {
            return Ok(());
        }
        if changed && read_text(&path)? != document.current {
            return Err(self.invalid_layout());
        }
        if changed && current.is_some() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &self.agent,
                &self.root,
                &path,
            )?;
        }
        if changed {
            if track {
                self.save_state(&InstructionState {
                    saved: old,
                    pending: Some(done.saved.clone()),
                })?;
            }
            fsx::atomic_write(&path, updated.as_bytes())?;
        }
        if track {
            self.save_state(&done)?;
        }
        Ok(())
    }

    pub(super) fn reconcile(&self) -> Result<()> {
        let document = self.load()?;
        let mut updated = document.text.clone();
        if let Some((start, end)) = block_span(&updated)? {
            if updated[start..end].lines().nth(1)
                == Some("## Project memory (managed by nativemem)")
            {
                let after = &updated[end..];
                let len = if after.starts_with("\r\n") {
                    2
                } else {
                    usize::from(after.starts_with('\n'))
                };
                updated.replace_range(start..end + len, "");
            }
        }
        let expected = document.anchors.clone();
        self.store(document, &updated, expected)
    }

    pub fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        self.require_global(item, "write_replica")?;
        let document = self.load()?;
        let origin = Origin {
            agent: key.origin.as_str().to_string(),
            id: key.native_id.clone(),
        };
        let native_id = replica_native_id(self.file, &origin.agent, &origin.id);
        let mut expected = document.anchors.clone();
        let heading = item.description.trim();
        let anchor = Anchor::new(heading, &trim_body(&item.body), Some(origin.clone()));
        let updated = if let Some(index) = expected
            .iter()
            .position(|a| a.origin.as_ref() == Some(&origin))
        {
            let raw = &document.sections[index];
            let mut updated = rewrite_section_body(&document.text, raw, &item.body);
            let end = raw.start
                + document.text[raw.start..raw.heading_end]
                    .trim_end_matches(['\r', '\n'])
                    .len();
            if heading != raw.heading {
                updated.replace_range(raw.start..end, &format!("{HEADING_PREFIX}{heading}"));
            }
            expected[index] = anchor;
            updated
        } else {
            expected.push(anchor);
            insert_at(&document.text, None, heading, &item.body)
        };
        self.store(document, &updated, expected)?;
        Ok((native_id, self.path()))
    }

    /// Appends `item` as a new user section; see `insert_section`. Fails with `Error::Exists`
    /// when a section with the same id is there.
    pub fn create_original(&self, item: &Item) -> Result<(String, PathBuf)> {
        self.require_global(item, "create")?;
        if let Some(problem) = section_problem(&item.description, &item.body) {
            return Err(Error::Parse(problem.to_string()));
        }
        let native_id = format!("{}#{}", self.file, slugify(item.description.trim()));
        let document = self.load()?;
        if document.ids(self.file).contains(&native_id) {
            return Err(Error::Exists(native_id));
        }
        let at = managed_spans(&document.text)?
            .first()
            .map(|&(s, _)| s)
            .into_iter()
            .chain(
                document
                    .anchors
                    .iter()
                    .zip(&document.sections)
                    .find_map(|(a, raw)| a.origin.as_ref().map(|_| raw.start)),
            )
            .min();
        let index = at.map_or(document.sections.len(), |at| {
            document.sections.partition_point(|raw| raw.start < at)
        });
        let mut expected = document.anchors.clone();
        expected.insert(
            index,
            Anchor::new(item.description.trim(), &trim_body(&item.body), None),
        );
        let updated = insert_at(&document.text, at, &item.description, &item.body);
        self.store(document, &updated, expected)?;
        Ok((native_id, self.path()))
    }

    fn missing(&self, native_id: &str) -> Error {
        Error::Parse(format!(
            "no instruction section {native_id:?} in {}",
            self.file
        ))
    }

    pub fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        self.require_global(item, "write_original")?;
        if classify(self.file, native_id) != Some(IdKind::Original) {
            return Err(self.missing(native_id));
        }
        let document = self.load()?;
        let index = document
            .ids(self.file)
            .iter()
            .position(|id| id == native_id)
            .ok_or_else(|| self.missing(native_id))?;
        let raw = &document.sections[index];
        let updated = rewrite_section_body(&document.text, raw, &item.body);
        let mut expected = document.anchors.clone();
        expected[index] = Anchor::new(&raw.heading, &trim_body(&item.body), None);
        self.store(document, &updated, expected)
    }

    pub fn remove(&self, native_id: &str) -> Result<()> {
        let document = self.load()?;
        let index = document
            .ids(self.file)
            .iter()
            .position(|id| id == native_id)
            .ok_or_else(|| self.missing(native_id))?;
        let updated = remove_at(&document.text, &document.sections[index]);
        let mut expected = document.anchors.clone();
        expected.remove(index);
        self.store(document, &updated, expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "CLAUDE.md";

    fn ids(text: &str) -> Vec<String> {
        located(text, FILE)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    fn sections(text: &str) -> Vec<(String, String, String)> {
        located(text, FILE)
            .unwrap()
            .into_iter()
            .map(|(id, r)| (id, r.heading.clone(), section_body(text, &r)))
            .collect()
    }

    #[test]
    fn splits_at_level_two_headings_and_keeps_deeper_ones_in_the_body() {
        let text = "# Title\n@RTK.md\n\n## One\nbody 1\n\n### Sub\nmore\n\n## Two\n\nbody 2\n";
        assert_eq!(
            sections(text),
            vec![
                (
                    "CLAUDE.md#one".into(),
                    "One".into(),
                    "body 1\n\n### Sub\nmore".into()
                ),
                ("CLAUDE.md#two".into(), "Two".into(), "body 2".into()),
            ]
        );
    }

    #[test]
    fn headings_inside_fences_do_not_split() {
        let text = "## A\n```md\n## not a heading\n```\n~~~\n## nor this\n~~~~\n## B\nx\n";
        assert_eq!(ids(text), ["CLAUDE.md#a", "CLAUDE.md#b"]);
        let a = &sections(text)[0];
        assert!(a.2.contains("## not a heading") && a.2.contains("## nor this"));
    }

    #[test]
    fn longer_fence_needs_a_matching_close() {
        let text = "## A\n````\n```\n## inside\n````\n## B\n";
        assert_eq!(ids(text), ["CLAUDE.md#a", "CLAUDE.md#b"]);
    }

    #[test]
    fn duplicate_headings_get_numbered_ids() {
        let text = "## Rules\na\n## Rules\nb\n## Rules!\nc\n";
        assert_eq!(
            ids(text),
            ["CLAUDE.md#rules", "CLAUDE.md#rules-2", "CLAUDE.md#rules-3"]
        );
    }

    #[test]
    fn preamble_and_empty_headings_are_not_sections() {
        assert!(ids("# Title\n@RTK.md\nintro\n").is_empty());
        assert!(ids("").is_empty());
        let text = "## \n## real\nx\n";
        assert_eq!(ids(text), ["CLAUDE.md#real"]);
        assert!(sections(text)[0].2 == "x");
    }

    #[test]
    fn empty_body_and_heading_at_end_of_file() {
        let text = "## A\n\n## B";
        let s = sections(text);
        assert_eq!(s[0].2, "");
        assert_eq!(s[1].2, "");
    }

    #[test]
    fn managed_regions_are_not_sections_and_end_a_section() {
        let text = format!(
            "## Mine\nbody\n\n{BLOCK_BEGIN}\n## Project memory (managed by nativemem)\n- x\n{BLOCK_END}\n\n## Later\nl\n\n{REGION_BEGIN}\n{}\n## Replica\n\nr\n{REGION_END}\n",
            marker("claude", "CLAUDE.md#replica")
        );
        assert_eq!(ids(&text), ["CLAUDE.md#mine", "CLAUDE.md#later"]);
        assert_eq!(sections(&text)[0].2, "body");
        assert_eq!(sections(&text)[1].2, "l");
    }

    #[test]
    fn unterminated_region_is_an_error() {
        assert!(located(&format!("## A\n{REGION_BEGIN}\n"), FILE).is_err());
    }

    #[test]
    fn rewrite_body_keeps_every_other_byte() {
        let text = "pre\n\n## A\nold a\n\n## B\nold b\n\ntail\n";
        let out = rewrite_body(text, FILE, "CLAUDE.md#a", "new a\nline 2\n")
            .unwrap()
            .unwrap();
        assert_eq!(out, "pre\n\n## A\nnew a\nline 2\n\n## B\nold b\n\ntail\n");
    }

    #[test]
    fn rewrite_body_fills_an_empty_section() {
        let out = rewrite_body("## A\n\n## B\n", FILE, "CLAUDE.md#a", "x")
            .unwrap()
            .unwrap();
        assert_eq!(out, "## A\n\nx\n\n## B\n");
        let out = rewrite_body("## A", FILE, "CLAUDE.md#a", "x")
            .unwrap()
            .unwrap();
        assert_eq!(out, "## A\n\nx\n");
        assert_eq!(
            rewrite_body("## A\n", FILE, "CLAUDE.md#a", "")
                .unwrap()
                .unwrap(),
            "## A\n"
        );
        assert!(rewrite_body("## A\n", FILE, "CLAUDE.md#zzz", "x")
            .unwrap()
            .is_none());
    }

    #[test]
    fn remove_section_cases() {
        let text = "pre\n\n## A\na\n\n## B\nb\n\n## C\nc\n";
        assert_eq!(
            remove_section(text, FILE, "CLAUDE.md#b").unwrap().unwrap(),
            "pre\n\n## A\na\n\n## C\nc\n"
        );
        assert_eq!(
            remove_section(text, FILE, "CLAUDE.md#c").unwrap().unwrap(),
            "pre\n\n## A\na\n\n## B\nb\n"
        );
        assert_eq!(
            remove_section("## A\na\n", FILE, "CLAUDE.md#a")
                .unwrap()
                .unwrap(),
            ""
        );
    }

    fn marked(origin: &str, id: &str, heading: &str, body: &str) -> String {
        format!(
            "{REGION_BEGIN}\n{}\n{}{REGION_END}\n",
            marker(origin, id),
            render_section(heading, body)
        )
    }

    fn instruction_file(base: &Path) -> InstructionFile {
        InstructionFile::new(
            AgentId::new("claude"),
            base.join("claude"),
            base.join("state"),
            BackupSession::with_stamp("T"),
            FILE,
        )
    }

    fn replica_key() -> ItemKey {
        ItemKey {
            origin: AgentId::new("pi"),
            native_id: "AGENTS.md#same".into(),
        }
    }

    #[test]
    fn equal_headings_can_be_reordered_and_the_replica_edited() {
        let dir = tempfile::tempdir().unwrap();
        let file = instruction_file(dir.path());
        let path = file.path();
        let original = "# Preamble\n\n## Same\n\nAn original.\n";
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, original).unwrap();
        let key = replica_key();
        file.write_replica(
            &key,
            &InstructionFile::item("Same".into(), "A copy.".into()),
        )
        .unwrap();
        fs::write(
            &path,
            "# Preamble\n\n## Same\n\nA changed copy.\n\n## Same\n\nAn original.\n",
        )
        .unwrap();
        let found = file.scan().unwrap();
        assert_eq!(found[0].replica_of, Some(key));
        assert_eq!(found[0].item.body, "A changed copy.");
        assert!(found[1].replica_of.is_none());
        assert_eq!(found[1].native_id, "CLAUDE.md#same");
    }

    #[test]
    fn disappearing_indistinguishable_section_does_not_adopt_or_delete() {
        let dir = tempfile::tempdir().unwrap();
        let file = instruction_file(dir.path());
        let path = file.path();
        let original = "## Same\n\nIdentical.\n";
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, original).unwrap();
        file.write_replica(
            &replica_key(),
            &InstructionFile::item("Same".into(), "Identical.".into()),
        )
        .unwrap();
        fs::write(&path, original).unwrap();
        assert!(matches!(file.scan(), Err(Error::Parse(_))));
        assert!(file.reconcile().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn interrupted_write_recovers_replica_identity_at_either_document_hash() {
        let dir = tempfile::tempdir().unwrap();
        let file = instruction_file(dir.path());
        let path = file.path();
        let before = "## Same\n\nFirst.\n";
        let after = "## Same\n\nSecond.\n";
        let key = replica_key();
        let origin = Origin {
            agent: key.origin.to_string(),
            id: key.native_id.clone(),
        };
        let state = InstructionState {
            saved: Snapshot::new(
                before,
                vec![Anchor::new("Same", "First.", Some(origin.clone()))],
            ),
            pending: Some(Snapshot::new(
                after,
                vec![Anchor::new("Same", "Second.", Some(origin))],
            )),
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for (text, body) in [(before, "First."), (after, "Second.")] {
            file.save_state(&state).unwrap();
            fs::write(&path, text).unwrap();
            let found = file.scan().unwrap();
            assert_eq!(found[0].replica_of, Some(key.clone()));
            assert_eq!(found[0].item.body, body);
            file.reconcile().unwrap();
            let saved = file.load().unwrap().state.unwrap();
            assert_eq!(saved.saved.hash, sha_hex(text.as_bytes()));
            assert!(saved.pending.is_none());
        }
    }

    #[test]
    fn replica_ids_with_odd_characters_round_trip() {
        let text = marked("codex", "AGENTS.md#a b:c%", "H", "b");
        assert_eq!(parse_region(&text).unwrap()[0].id, "AGENTS.md#a b:c%");
    }

    #[test]
    fn insert_section_goes_before_the_first_managed_region_and_keeps_every_other_byte() {
        let user = "# Mine\n\n## Style\nbe terse\n";
        let region = format!("{user}\n{}", marked("codex", "AGENTS.md#z", "Z", "zed"));
        let out = insert_section(&region, FILE, "CLAUDE.md#new", "New", "fresh body\n")
            .unwrap()
            .unwrap();
        let at = region.find(REGION_BEGIN).unwrap();
        let (before, after) = region.split_at(at);
        assert_eq!(out, format!("{before}## New\n\nfresh body\n\n{after}"));
        assert_eq!(ids(&out), ["CLAUDE.md#style", "CLAUDE.md#new"]);

        let with_block =
            format!("{user}\n{BLOCK_BEGIN}\n- x\n{BLOCK_END}\n\n{REGION_BEGIN}\n{REGION_END}\n");
        let out = insert_section(&with_block, FILE, "CLAUDE.md#new", "New", "n")
            .unwrap()
            .unwrap();
        assert!(out.starts_with(&format!("{user}\n## New\n\nn\n\n{BLOCK_BEGIN}\n")));
        assert!(out.ends_with(&with_block[user.len() + 1..]));
    }

    #[test]
    fn insert_section_without_a_region_appends_with_one_blank_line() {
        let cases = [
            ("", "## New\n\nb\n"),
            ("# T\n", "# T\n\n## New\n\nb\n"),
            ("# T\n\n", "# T\n\n## New\n\nb\n"),
            ("# T", "# T\n\n## New\n\nb\n"),
        ];
        for (text, expected) in cases {
            let out = insert_section(text, FILE, "CLAUDE.md#new", "New", "b")
                .unwrap()
                .unwrap();
            assert_eq!(out, expected, "{text:?}");
        }
        let empty_body = insert_section("# T\n", FILE, "CLAUDE.md#new", "New", "")
            .unwrap()
            .unwrap();
        assert_eq!(empty_body, "# T\n\n## New\n");
    }

    #[test]
    fn insert_section_refuses_an_id_that_is_taken() {
        let text = "## Style\nx\n\n## Style!\ny\n";
        assert!(insert_section(text, FILE, "CLAUDE.md#style", "Style", "z")
            .unwrap()
            .is_none());
        assert!(
            insert_section(text, FILE, "CLAUDE.md#style-2", "Style 2", "z")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn section_problem_rejects_what_would_not_read_back() {
        assert_eq!(section_problem("Style", "- a\n\n### Sub\nb"), None);
        assert_eq!(section_problem("Style", "```\n## in a fence\n```"), None);
        assert!(section_problem("  ", "b").is_some());
        assert!(section_problem("two\nlines", "b").is_some());
        assert!(section_problem("Style", "a\n## Other\nb").is_some());
        assert!(section_problem("Style", &format!("a\n{REGION_BEGIN}")).is_some());
    }

    #[test]
    fn classify_ids() {
        assert_eq!(classify(FILE, "CLAUDE.md#a-2"), Some(IdKind::Original));
        assert_eq!(
            classify(FILE, "CLAUDE.md#codex:AGENTS.md#a"),
            Some(IdKind::Replica {
                origin: "codex".into(),
                id: "AGENTS.md#a".into()
            })
        );
        assert_eq!(classify(FILE, "-w-app/a.md"), None);
        assert_eq!(classify(FILE, "/w/app#slug"), None);
    }
}
