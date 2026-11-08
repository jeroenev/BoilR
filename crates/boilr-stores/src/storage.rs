//! Sessions and libraries live in `<config folder>/stores/<store key>/`. Session files hold
//! tokens and cookies, so they are created readable by the user only (mode 0600 on Unix;
//! on Windows the config folder is under the user's own `%APPDATA%`).

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Serialize};

use crate::{OwnedLibrary, Session, StoreId};

fn store_folder(store: StoreId) -> PathBuf {
    stores_root().join(store.key())
}

fn stores_root() -> PathBuf {
    // Tests point this at a temporary folder.
    if let Some(dir) = std::env::var_os("BOILR_STORES_DIR") {
        return PathBuf::from(dir);
    }
    boilr_core::config::get_config_folder().join("stores")
}

fn session_file(store: StoreId) -> PathBuf {
    store_folder(store).join("session.json")
}

fn library_file(store: StoreId) -> PathBuf {
    store_folder(store).join("library.json")
}

pub fn load_session(store: StoreId) -> eyre::Result<Option<Session>> {
    read_json(&session_file(store))
}

pub fn save_session(store: StoreId, session: &Session) -> eyre::Result<()> {
    write_json(&session_file(store), session, true)
}

pub fn load_library(store: StoreId) -> eyre::Result<Option<OwnedLibrary>> {
    read_json(&library_file(store))
}

pub fn save_library(store: StoreId, library: &OwnedLibrary) -> eyre::Result<()> {
    write_json(&library_file(store), library, false)
}

/// Signs out: removes the session and the cached library.
pub fn delete_account(store: StoreId) -> eyre::Result<()> {
    let folder = store_folder(store);
    if folder.exists() {
        fs::remove_dir_all(&folder)?;
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> eyre::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Writes to a temporary file and renames it over the target, so a crash never leaves half a file.
fn write_json<T: Serialize>(path: &Path, value: &T, private: bool) -> eyre::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| eyre::format_err!("{} has no parent folder", path.display()))?;
    fs::create_dir_all(folder)?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    {
        let mut file = open_new(&tmp, private)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn open_new(path: &Path, private: bool) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if private {
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(not(unix))]
fn open_new(path: &Path, _private: bool) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OwnedGame;

    #[test]
    fn round_trips_session_and_library() -> eyre::Result<()> {
        let dir = std::env::temp_dir().join(format!("boilr-stores-test-{}", std::process::id()));
        std::env::set_var("BOILR_STORES_DIR", &dir);

        assert!(load_session(StoreId::Gog)?.is_none());
        let session = Session {
            account_name: Some("player".into()),
            data: serde_json::json!({"refresh_token": "abc"}),
        };
        save_session(StoreId::Gog, &session)?;
        let loaded = load_session(StoreId::Gog)?;
        assert_eq!(
            loaded.and_then(|s| s.account_name),
            Some("player".to_string())
        );

        let library = OwnedLibrary {
            account_name: None,
            fetched_at: 1,
            games: vec![OwnedGame::new(StoreId::Gog, "1207658924", "Unreal Gold")],
        };
        save_library(StoreId::Gog, &library)?;
        assert_eq!(load_library(StoreId::Gog)?.map(|l| l.games.len()), Some(1));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(session_file(StoreId::Gog))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        delete_account(StoreId::Gog)?;
        assert!(load_session(StoreId::Gog)?.is_none());
        let _ = fs::remove_dir_all(dir);
        Ok(())
    }
}
