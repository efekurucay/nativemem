//! Instruction sections of a global instruction file (`CLAUDE.md`, `AGENTS.md`): the section
//! parser, the managed regions and the byte-preserving edits shared by both adapters.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{AgentId, Item, ItemKey, Kind, Observed, Scope};
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

struct Raw {
    heading: String,
    start: usize,
    heading_end: usize,
    end: usize,
}

fn fence_run(line: &str) -> Option<(char, usize, &str)> {
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
fn located(text: &str, file: &str) -> Result<Vec<(String, Raw)>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for raw in raw_sections(text, &managed_spans(text)?) {
        let base = format!("{file}#{}", slugify(&raw.heading));
        let mut id = base.clone();
        let mut n = 2;
        while seen.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        seen.push(id.clone());
        out.push((id, raw));
    }
    Ok(out)
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

fn oneline(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn render_region(replicas: &[Replica]) -> String {
    let mut s = format!("{REGION_BEGIN}\n");
    for (i, r) in replicas.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(&format!(
            "{}\n{HEADING_PREFIX}{}\n",
            marker(&r.origin, &r.id),
            oneline(&r.heading)
        ));
        if !r.body.is_empty() {
            s.push_str(&format!("\n{}\n", r.body));
        }
    }
    s.push_str(REGION_END);
    s
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

/// Replaces the instruction region with one holding `replicas` (sorted by origin, id), or
/// removes it when there are none. Bytes outside the region stay as they are.
fn set_region(text: &str, mut replicas: Vec<Replica>) -> Result<String> {
    replicas.sort_by(|a, b| (&a.origin, &a.id).cmp(&(&b.origin, &b.id)));
    Ok(match (region_span(text)?, replicas.is_empty()) {
        (Some((s, e)), false) => {
            format!("{}{}{}", &text[..s], render_region(&replicas), &text[e..])
        }
        (Some((s, e)), true) => {
            let mut before = &text[..s];
            if before.ends_with("\n\n") {
                before = &before[..before.len() - 1];
            }
            let after = text[e..].strip_prefix('\n').unwrap_or(&text[e..]);
            format!("{before}{after}")
        }
        (None, false) => {
            let sep = if text.is_empty() {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{text}{sep}{}\n", render_region(&replicas))
        }
        (None, true) => text.to_string(),
    })
}

pub fn upsert_replica(text: &str, replica: Replica) -> Result<String> {
    let mut replicas = parse_region(text)?;
    match replicas
        .iter_mut()
        .find(|r| r.origin == replica.origin && r.id == replica.id)
    {
        Some(r) => *r = replica,
        None => replicas.push(replica),
    }
    set_region(text, replicas)
}

/// `None` when the region holds no such replica.
pub fn remove_replica(text: &str, origin: &str, id: &str) -> Result<Option<String>> {
    let mut replicas = parse_region(text)?;
    let before = replicas.len();
    replicas.retain(|r| !(r.origin == origin && r.id == id));
    if replicas.len() == before {
        return Ok(None);
    }
    set_region(text, replicas).map(Some)
}

/// Replaces the body of an original section in place; the heading line and every other byte
/// stay. `None` when the section does not exist.
pub fn rewrite_body(text: &str, file: &str, native_id: &str, body: &str) -> Result<Option<String>> {
    let sections = located(text, file)?;
    let Some((_, raw)) = sections.iter().find(|(id, _)| id == native_id) else {
        return Ok(None);
    };
    let body = trim_body(body);
    Ok(Some(match body_range(text, raw) {
        Some((s, e)) => format!("{}{body}{}", &text[..s], &text[e..]),
        None if body.is_empty() => text.to_string(),
        None => {
            let head = &text[..raw.heading_end];
            let nl = if head.ends_with('\n') { "" } else { "\n" };
            format!("{head}{nl}\n{body}\n{}", &text[raw.heading_end..])
        }
    }))
}

/// Deletes an original section including the blank lines after it. `None` when the section
/// does not exist.
pub fn remove_section(text: &str, file: &str, native_id: &str) -> Result<Option<String>> {
    let sections = located(text, file)?;
    let Some((_, raw)) = sections.iter().find(|(id, _)| id == native_id) else {
        return Ok(None);
    };
    let (head, tail) = (&text[..raw.start], &text[raw.end..]);
    Ok(Some(if tail.is_empty() && !head.trim().is_empty() {
        format!("{}\n", head.trim_end())
    } else if tail.is_empty() {
        String::new()
    } else {
        format!("{head}{tail}")
    }))
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

    fn item(heading: String, body: String) -> Item {
        Item {
            kind: Kind::Instruction,
            scope: Scope::Global,
            name: slugify(&heading),
            description: heading,
            category: None,
            body,
        }
    }

    pub fn scan(&self) -> Result<Vec<Observed>> {
        let Some(text) = read_text(&self.path())? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (native_id, raw) in located(&text, self.file)? {
            let body = section_body(&text, &raw);
            out.push(Observed {
                agent: self.agent.clone(),
                native_id,
                path: self.path(),
                item: Self::item(raw.heading, body),
                replica_of: None,
            });
        }
        for r in parse_region(&text)? {
            out.push(Observed {
                agent: self.agent.clone(),
                native_id: replica_native_id(self.file, &r.origin, &r.id),
                path: self.path(),
                item: Self::item(r.heading, r.body),
                replica_of: Some(ItemKey {
                    origin: AgentId::new(r.origin),
                    native_id: r.id,
                }),
            });
        }
        Ok(out)
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

    /// Writes `updated` when it differs from `current`, backing up an existing file first.
    fn store(&self, current: Option<&str>, updated: &str) -> Result<()> {
        if current == Some(updated) {
            return Ok(());
        }
        let path = self.path();
        if current.is_some() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &self.agent,
                &self.root,
                &path,
            )?;
        }
        fsx::atomic_write(&path, updated.as_bytes())
    }

    pub fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        self.require_global(item, "write_replica")?;
        let current = read_text(&self.path())?;
        let updated = upsert_replica(
            current.as_deref().unwrap_or_default(),
            Replica {
                origin: key.origin.as_str().to_string(),
                id: key.native_id.clone(),
                heading: item.description.clone(),
                body: trim_body(&item.body),
            },
        )?;
        self.store(current.as_deref(), &updated)?;
        Ok((
            replica_native_id(self.file, key.origin.as_str(), &key.native_id),
            self.path(),
        ))
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
        let current = read_text(&self.path())?.ok_or_else(|| self.missing(native_id))?;
        let updated = rewrite_body(&current, self.file, native_id, &item.body)?
            .ok_or_else(|| self.missing(native_id))?;
        self.store(Some(&current), &updated)
    }

    pub fn remove(&self, native_id: &str) -> Result<()> {
        let kind = classify(self.file, native_id).ok_or_else(|| self.missing(native_id))?;
        let current = read_text(&self.path())?.ok_or_else(|| self.missing(native_id))?;
        let updated = match kind {
            IdKind::Original => remove_section(&current, self.file, native_id)?,
            IdKind::Replica { origin, id } => remove_replica(&current, &origin, &id)?,
        }
        .ok_or_else(|| self.missing(native_id))?;
        self.store(Some(&current), &updated)
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

    fn rep(origin: &str, id: &str, heading: &str, body: &str) -> Replica {
        Replica {
            origin: origin.into(),
            id: id.into(),
            heading: heading.into(),
            body: body.into(),
        }
    }

    #[test]
    fn region_is_created_sorted_updated_and_removed_with_its_blank_line() {
        let base = "# Mine\n\nrules\n";
        let one = upsert_replica(base, rep("codex", "AGENTS.md#z", "Z", "zed")).unwrap();
        let two = upsert_replica(&one, rep("claude", "CLAUDE.md#a", "A", "")).unwrap();
        assert!(two.starts_with(&format!("{base}\n{REGION_BEGIN}\n")));
        assert!(two.ends_with(&format!("{REGION_END}\n")));
        assert!(two.find("origin=claude").unwrap() < two.find("origin=codex").unwrap());
        let parsed = parse_region(&two).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].body, "");
        assert_eq!(parsed[1], rep("codex", "AGENTS.md#z", "Z", "zed"));

        let updated = upsert_replica(&two, rep("codex", "AGENTS.md#z", "Z", "zed 2")).unwrap();
        assert_eq!(parse_region(&updated).unwrap().len(), 2);
        assert!(updated.contains("zed 2"));

        let one_left = remove_replica(&two, "claude", "CLAUDE.md#a")
            .unwrap()
            .unwrap();
        let none_left = remove_replica(&one_left, "codex", "AGENTS.md#z")
            .unwrap()
            .unwrap();
        assert_eq!(none_left, base);
        assert!(remove_replica(&none_left, "codex", "x").unwrap().is_none());
    }

    #[test]
    fn region_alone_in_a_new_file() {
        let text = upsert_replica("", rep("claude", "CLAUDE.md#a", "A", "a")).unwrap();
        assert!(text.starts_with(REGION_BEGIN));
        assert_eq!(
            remove_replica(&text, "claude", "CLAUDE.md#a")
                .unwrap()
                .unwrap(),
            ""
        );
    }

    #[test]
    fn replica_ids_with_odd_characters_round_trip() {
        let text = upsert_replica("", rep("codex", "AGENTS.md#a b:c%", "H", "b")).unwrap();
        assert_eq!(parse_region(&text).unwrap()[0].id, "AGENTS.md#a b:c%");
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
