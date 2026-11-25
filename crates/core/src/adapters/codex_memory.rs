//! Project notes in Codex's ad-hoc memory input. Codex owns the generated summary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{format_iso_ms, sha_hex};
use crate::slug::slugify;

use super::memory_store::{parse_entry, project_key, read_opt, render, MemoryStore};

const AGENT: &str = "codex";
const NOTE_SCOPE: &str = "Project: ";
const FORGET: &str = "# Forget a project note";

#[derive(Serialize, Deserialize)]
struct Note {
    file: String,
    store: String,
    origin: Option<(String, String)>,
    #[serde(default)]
    published: bool,
}

type Catalog = BTreeMap<String, Note>;

pub(super) struct Notes {
    home: PathBuf,
    state: PathBuf,
    backups: BackupSession,
    catalog: PathBuf,
    manager: MemoryStore,
}

impl Notes {
    pub(super) fn new(home: &Path, state: &Path, backups: BackupSession) -> Self {
        let resources = home.join("memories/extensions/nativemem/resources");
        let identity = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        Self {
            home: home.to_path_buf(),
            state: state.to_path_buf(),
            catalog: state.join("codex-notes").join(format!(
                "{}.json",
                sha_hex(identity.as_os_str().as_encoded_bytes())
            )),
            manager: MemoryStore::new(
                AgentId::new(AGENT),
                home.to_path_buf(),
                state.to_path_buf(),
                backups.clone(),
                resources.clone(),
                resources,
            ),
            backups,
        }
    }

    fn load(&self) -> Result<Catalog> {
        let Some(text) = read_opt(&self.catalog)? else {
            return Ok(Catalog::new());
        };
        serde_json::from_str(&text)
            .map_err(|e| Error::Parse(format!("invalid Codex note catalog: {e}")))
    }

    fn active_store(&self) -> Result<&'static str> {
        let config = read_opt(&self.home.join("config.toml"))?.unwrap_or_default();
        let config: toml::Table = toml::from_str(&config).map_err(|_| {
            Error::Parse("invalid Codex config; cannot select the memory store".into())
        })?;
        let version = config
            .get("memories")
            .and_then(|memory| memory.get("version"));
        let dual_write_disabled = config
            .get("memories")
            .and_then(|memory| memory.get("dual_write"))
            .and_then(toml::Value::as_bool)
            == Some(false);
        match version.and_then(toml::Value::as_str) {
            Some("v1") => Ok("memories"),
            Some("v2") => Ok("memories_v2"),
            None if version.is_none()
                && (dual_write_disabled || self.home.join("memories").is_dir()) =>
            {
                Ok("memories")
            }
            None if version.is_none() => Ok("memories_v2"),
            _ => Err(Error::unsupported(AGENT, "unrecognized memory store")),
        }
    }

    pub(super) fn generated_path(&self) -> Result<PathBuf> {
        Ok(self.home.join(self.active_store()?).join("MEMORY.md"))
    }

    fn note_path(&self, note: &Note) -> Result<PathBuf> {
        if !matches!(note.store.as_str(), "memories" | "memories_v2")
            || !note.file.ends_with(".md")
            || !note.file[..note.file.len() - 3]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(Error::Parse("invalid Codex note path".into()));
        }
        let mut path = self.home.clone();
        for part in [
            note.store.as_str(),
            "extensions",
            "ad_hoc",
            "notes",
            &note.file,
        ] {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_symlink() => {
                    return Err(Error::unsupported(AGENT, "linked note path"))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(&path, e)),
            }
        }
        Ok(path)
    }

    fn inspect(&self, id: &str, note: &Note) -> Result<Option<Observed>> {
        let path = self.note_path(note)?;
        let Some(text) = read_opt(&path)? else {
            return Ok(None);
        };
        if text.starts_with(FORGET) {
            return Ok(None);
        }
        let Some((scope, body)) = text.split_once("\n\n") else {
            return Ok(None);
        };
        let Some(cwd) = scope
            .strip_prefix(NOTE_SCOPE)
            .and_then(|raw| serde_json::from_str::<PathBuf>(raw).ok())
        else {
            return Ok(None);
        };
        let Some(entry) = parse_entry(body) else {
            return Ok(None);
        };
        Ok(Some(Observed {
            agent: AgentId::new(AGENT),
            native_id: id.into(),
            path,
            item: Item {
                kind: Kind::Memory,
                scope: Scope::Project(cwd),
                name: entry.name,
                description: entry.description,
                category: entry.category,
                body: entry.body,
                source_dir: None,
            },
            replica_of: note.origin.as_ref().map(|(agent, id)| ItemKey {
                origin: AgentId::new(agent),
                native_id: id.clone(),
            }),
        }))
    }

    pub(super) fn scan(&self) -> Result<Vec<Observed>> {
        let catalog = self.load()?;
        let mut items = Vec::new();
        for (id, note) in &catalog {
            if let Some(observed) = self.inspect(id, note)? {
                items.push(observed);
            }
        }
        for item in self.manager.scan()? {
            if !self.has_native_note(&catalog, &item.native_id)? {
                items.push(item);
            }
        }
        Ok(items)
    }

    /// Preserve the logical id and source key when adopting a manager note into Codex's input.
    pub(super) fn adopt_manager_notes(&self) -> Result<()> {
        let mut catalog = self.load()?;
        for entry in self.manager.scan()? {
            if let Some(note) = catalog.get_mut(&entry.native_id) {
                if note.published {
                    continue;
                }
                if self.note_path(note)?.is_file() {
                    note.published = true;
                    self.save(&catalog)?;
                    continue;
                }
            }
            self.write(
                &mut catalog,
                &entry.native_id,
                entry.replica_of.as_ref(),
                &entry.item,
            )?;
        }
        Ok(())
    }

    fn has_native_note(&self, catalog: &Catalog, id: &str) -> Result<bool> {
        catalog
            .get(id)
            .map(|note| {
                self.note_path(note)
                    .map(|path| note.published || path.is_file())
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn write(
        &self,
        catalog: &mut Catalog,
        id: &str,
        key: Option<&ItemKey>,
        item: &Item,
    ) -> Result<PathBuf> {
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported(AGENT, "write global memory"));
        };
        let text = format!(
            "{NOTE_SCOPE}{}\n\n{}",
            serde_json::to_string(cwd).map_err(|e| Error::Parse(e.to_string()))?,
            render(None, item)
        );
        let origin = key.map(|key| (key.origin.as_str().to_string(), key.native_id.clone()));
        if let Some(note) = catalog.get(id) {
            if note.origin != origin {
                return Err(Error::Exists(id.into()));
            }
            let path = self.note_path(note)?;
            self.write_note(&path, &text)?;
            if !note.published {
                catalog.get_mut(id).expect("note exists").published = true;
                self.save(catalog)?;
            }
            return Ok(path);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let stamp = format_iso_ms(now)[..19].replace(':', "-");
        let digest = sha_hex(id.as_bytes());
        let note = Note {
            file: format!("{stamp}-{}.md", &digest[..32]),
            store: self.active_store()?.into(),
            origin,
            published: false,
        };
        let path = self.note_path(&note)?;
        if path.exists() {
            return Err(Error::Exists("Codex note file".into()));
        }
        catalog.insert(id.to_string(), note);
        self.save(catalog)?;
        self.write_note(&path, &text)?;
        catalog.get_mut(id).expect("note exists").published = true;
        self.save(catalog)?;
        Ok(path)
    }

    fn save(&self, catalog: &Catalog) -> Result<()> {
        if self.catalog.exists() {
            fsx::backup(
                &self.backups,
                &self.state,
                &AgentId::new(AGENT),
                &self.state,
                &self.catalog,
            )?;
        }
        let data = serde_json::to_vec(catalog).map_err(|e| Error::Parse(e.to_string()))?;
        fsx::atomic_write(&self.catalog, &data)
    }

    fn write_note(&self, path: &Path, text: &str) -> Result<()> {
        if let Some(existing) = read_opt(path)? {
            if existing == text {
                return Ok(());
            }
            fsx::backup(
                &self.backups,
                &self.state,
                &AgentId::new(AGENT),
                &self.home,
                path,
            )?;
        }
        fsx::atomic_write(path, text.as_bytes())
    }

    pub(super) fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        let mut catalog = self.load()?;
        let existing = catalog.iter().find_map(|(id, note)| {
            note.origin
                .as_ref()
                .filter(|(agent, native_id)| {
                    agent == key.origin.as_str() && native_id == &key.native_id
                })
                .map(|_| id.clone())
        });
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported(AGENT, "write global memory"));
        };
        let digest = sha_hex(format!("{}:{}", key.origin, key.native_id).as_bytes());
        let id = existing.unwrap_or_else(|| format!("{}/{}.md", project_key(cwd), &digest[..32]));
        let path = self.write(&mut catalog, &id, Some(key), item)?;
        Ok((id, path))
    }

    pub(super) fn create_original(&self, item: &Item) -> Result<(String, PathBuf)> {
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported(AGENT, "create global memory"));
        };
        let id = format!("{}/{}.md", project_key(cwd), slugify(&item.name));
        let mut catalog = self.load()?;
        if catalog.get(&id).is_some_and(|note| note.origin.is_some())
            || self.scan()?.iter().any(|note| note.native_id == id)
        {
            return Err(Error::Exists(id));
        }
        let path = self.write(&mut catalog, &id, None, item)?;
        Ok((id, path))
    }

    pub(super) fn write_original(&self, id: &str, item: &Item) -> Result<()> {
        let mut catalog = self.load()?;
        if let Some(note) = catalog.get(id).filter(|note| note.origin.is_none()) {
            if self.inspect(id, note)?.is_some() {
                return self.write(&mut catalog, id, None, item).map(|_| ());
            }
        }
        if !catalog.contains_key(id)
            && self
                .manager
                .scan()?
                .iter()
                .any(|note| note.native_id == id && note.replica_of.is_none())
        {
            return self.write(&mut catalog, id, None, item).map(|_| ());
        }
        Err(Error::unsupported(AGENT, "edit a Codex memory original"))
    }

    pub(super) fn remove(&self, id: &str) -> Result<()> {
        let catalog = self.load()?;
        let Some(note) = catalog.get(id) else {
            if self.manager.scan()?.iter().any(|note| note.native_id == id) {
                return self.manager.remove(id);
            }
            return Err(Error::unsupported(AGENT, "remove a Codex memory original"));
        };
        if let Some(entry) = self.inspect(id, note)? {
            let Scope::Project(cwd) = entry.item.scope else {
                unreachable!()
            };
            let text = format!(
                "{FORGET}\n\n{NOTE_SCOPE}{}\n\nForget the note {:?} about {:?} for this project.\n",
                serde_json::to_string(&cwd).map_err(|e| Error::Parse(e.to_string()))?,
                entry.item.name,
                entry.item.description
            );
            self.write_note(&entry.path, &text)?;
        }
        Ok(())
    }
}
