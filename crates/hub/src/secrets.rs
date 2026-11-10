use std::collections::BTreeMap;
use std::path::PathBuf;

use nativemem_core::fsx::atomic_write;

use crate::error::{HubError, Result};

/// Prefix of a config value that names a secret instead of holding it.
pub const SECRET_PREFIX: &str = "secret:";

/// The keychain service every secret is stored under.
pub const KEYCHAIN_SERVICE: &str = "nativemem";

/// The secret id a config value refers to, if it is a `secret:<id>` reference.
pub fn secret_ref(value: &str) -> Option<&str> {
    value.strip_prefix(SECRET_PREFIX)
}

/// Secret ids are file-name and keychain-account safe.
pub fn validate_secret_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(HubError::Invalid(format!(
            "secret id {id:?} must be 1-128 characters of A-Z a-z 0-9 - _ ."
        )))
    }
}

/// Where secret values live. Values leave a store only to be passed to an upstream server.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, value: &str) -> Result<()>;
    /// `false` when the secret did not exist.
    fn delete(&self, id: &str) -> Result<bool>;
}

/// The operating system keychain.
pub struct KeychainStore;

fn entry(id: &str) -> Result<keyring::Entry> {
    validate_secret_id(id)?;
    keyring::Entry::new(KEYCHAIN_SERVICE, id).map_err(|e| HubError::SecretStore(e.to_string()))
}

impl SecretStore for KeychainStore {
    fn get(&self, id: &str) -> Result<Option<String>> {
        match entry(id)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(HubError::SecretStore(e.to_string())),
        }
    }

    fn set(&self, id: &str, value: &str) -> Result<()> {
        entry(id)?
            .set_password(value)
            .map_err(|e| HubError::SecretStore(e.to_string()))
    }

    fn delete(&self, id: &str) -> Result<bool> {
        match entry(id)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(HubError::SecretStore(e.to_string())),
        }
    }
}

/// A JSON file `<dir>/secrets.json`, for tests. It lives outside the state directory and is not
/// backed up: a backup would copy the values into the backup tree.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileStore {
            path: dir.into().join("secrets.json"),
        }
    }

    fn read(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                HubError::SecretStore(format!("cannot parse {}: {e}", self.path.display()))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(nativemem_core::error::Error::io(&self.path, e).into()),
        }
    }

    fn write(&self, map: &BTreeMap<String, String>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(map)
            .map_err(|e| HubError::SecretStore(format!("cannot encode secrets: {e}")))?;
        atomic_write(&self.path, &bytes)?;
        Ok(())
    }
}

impl SecretStore for FileStore {
    fn get(&self, id: &str) -> Result<Option<String>> {
        validate_secret_id(id)?;
        Ok(self.read()?.get(id).cloned())
    }

    fn set(&self, id: &str, value: &str) -> Result<()> {
        validate_secret_id(id)?;
        let mut map = self.read()?;
        map.insert(id.to_string(), value.to_string());
        self.write(&map)
    }

    fn delete(&self, id: &str) -> Result<bool> {
        validate_secret_id(id)?;
        let mut map = self.read()?;
        let existed = map.remove(id).is_some();
        if existed {
            self.write(&map)?;
        }
        Ok(existed)
    }
}

/// A value with its `secret:<id>` reference replaced by the stored secret. The second element is
/// the secret value when the input was a reference, so callers can scrub it from messages.
pub fn resolve_value(store: &dyn SecretStore, value: &str) -> Result<(String, Option<String>)> {
    match secret_ref(value) {
        None => Ok((value.to_string(), None)),
        Some(id) => match store.get(id)? {
            Some(secret) => Ok((secret.clone(), Some(secret))),
            None => Err(HubError::Invalid(format!("secret {id:?} is not set"))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trips_and_deletes() {
        let t = tempfile::tempdir().unwrap();
        let store = FileStore::new(t.path());
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "one").unwrap();
        store.set("b", "two").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("one"));
        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap());
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.get("b").unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn ids_are_validated() {
        assert!(validate_secret_id("github-token.1").is_ok());
        for bad in ["", "a b", "a/b", "é"] {
            assert!(validate_secret_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn references_resolve_and_literals_pass_through() {
        let t = tempfile::tempdir().unwrap();
        let store = FileStore::new(t.path());
        store.set("tok", "s3cret").unwrap();
        assert_eq!(
            resolve_value(&store, "secret:tok").unwrap(),
            ("s3cret".to_string(), Some("s3cret".to_string()))
        );
        assert_eq!(
            resolve_value(&store, "plain").unwrap(),
            ("plain".to_string(), None)
        );
        let err = resolve_value(&store, "secret:missing").unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
    }
}
