//! Store accounts: sign in to a game store, keep the session, and list every game the
//! account owns, installed or not. Modelled on Playnite's library plugins
//! (github.com/JosefNemec/PlayniteExtensions, `source/Libraries`), which talk to the same
//! store services.
//!
//! This crate knows nothing about Steam or about UIs. A front end runs a login (a webview,
//! or the user pasting a value), hands the result to [`complete_login`], and later calls
//! [`refresh_library`]. The `boilr` crate turns the cached library into Steam shortcuts.
#![deny(clippy::unwrap_in_result)]
#![deny(clippy::get_unwrap)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]

pub mod amazon;
pub mod battlenet;
pub mod ea;
pub mod epic;
pub mod gog;
pub mod http;
pub mod humble;
pub mod itch;
mod storage;

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use serde::{Deserialize, Serialize};

pub use storage::{load_library, load_session, save_library, save_session};

/// A store BoilR can sign in to. The `key` is used in file names and settings; never change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreId {
    Epic,
    Gog,
    Amazon,
    Ea,
    BattleNet,
    Humble,
    Itch,
}

impl StoreId {
    pub const ALL: [StoreId; 7] = [
        StoreId::Epic,
        StoreId::Gog,
        StoreId::Amazon,
        StoreId::Ea,
        StoreId::BattleNet,
        StoreId::Humble,
        StoreId::Itch,
    ];

    /// Stable identifier, used for file names, settings sections and shortcut identities.
    pub fn key(&self) -> &'static str {
        match self {
            StoreId::Epic => "epic",
            StoreId::Gog => "gog",
            StoreId::Amazon => "amazon",
            StoreId::Ea => "ea",
            StoreId::BattleNet => "battlenet",
            StoreId::Humble => "humble",
            StoreId::Itch => "itch",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            StoreId::Epic => "Epic Games Store",
            StoreId::Gog => "GOG",
            StoreId::Amazon => "Amazon Games",
            StoreId::Ea => "EA app",
            StoreId::BattleNet => "Battle.net",
            StoreId::Humble => "Humble Bundle",
            StoreId::Itch => "itch.io",
        }
    }

    pub fn from_key(key: &str) -> Option<StoreId> {
        StoreId::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// One game the account owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedGame {
    pub store: StoreId,
    /// The store's stable id for the game, the same id its launcher and BoilR's installed-game
    /// platforms use, so an owned game and its installed copy can be matched:
    /// Epic `appName`, GOG product id, Amazon product id, EA offer id, Battle.net product
    /// code, Humble `machine_name`, itch game id.
    pub id: String,
    pub title: String,
    /// Store-specific values the install target needs (for example Epic's `namespace` and
    /// `catalog_item_id`, or an itch game's `url`). Keys are documented per store module.
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

impl OwnedGame {
    pub fn new(store: StoreId, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            store,
            id: id.into(),
            title: title.into(),
            extra: BTreeMap::new(),
        }
    }

    pub fn with_extra(mut self, key: &str, value: impl Into<String>) -> Self {
        self.extra.insert(key.to_string(), value.into());
        self
    }

    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extra.get(key).map(String::as_str)
    }
}

/// The owned games of one account, as last fetched.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OwnedLibrary {
    pub account_name: Option<String>,
    /// Unix time in seconds.
    pub fetched_at: u64,
    pub games: Vec<OwnedGame>,
}

/// A signed-in account. `data` is owned by the store module (tokens, cookies, ids) and is
/// stored as-is in the user's config folder, readable only by the user.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub account_name: Option<String>,
    pub data: serde_json::Value,
}

/// A cookie captured from a login webview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
}

/// How a login can be completed without a webview: the user signs in with their own browser
/// and pastes one value back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManualLogin {
    /// Not possible: this store needs the built-in login window.
    Unsupported,
    /// Open `url`, sign in, and paste what `instructions` describes.
    Paste {
        url: String,
        instructions: String,
        placeholder: String,
    },
}

/// Everything a front end needs to run a login, plus state the store needs to finish it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingLogin {
    pub store: StoreId,
    /// Where the login window starts.
    pub start_url: String,
    /// The login is done once the window navigates to a URL starting with one of these.
    /// The window should not load that URL; it hands it to [`complete_login`] instead.
    pub done_url_prefixes: Vec<String>,
    /// URLs whose cookies the window collects when the login is done.
    pub cookie_urls: Vec<String>,
    pub manual: ManualLogin,
    /// The user agent the login window should use, when the store's sign-in page needs a
    /// launcher's own (Epic's captcha only passes for the Epic Games Launcher's).
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Store-private state carried from [`pending_login`] to [`complete_login`] (PKCE
    /// verifiers, device serials).
    #[serde(default)]
    pub state: serde_json::Value,
}

/// What a login produced.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoginCapture {
    /// The URL that matched `done_url_prefixes` (webview login).
    pub final_url: Option<String>,
    /// Cookies for `cookie_urls` (webview login).
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    /// The value the user pasted (manual login).
    pub pasted: Option<String>,
}

/// The sign-in in progress per store. A login started in one place (a window, a link the
/// user opens in their browser) is finished in another (a pasted value), so both must use
/// the same [`PendingLogin`]: Amazon's PKCE verifier and device serial live in its state.
static PENDING_LOGINS: Mutex<BTreeMap<StoreId, PendingLogin>> = Mutex::new(BTreeMap::new());

/// The sign-in in progress for `store`, started on first use. [`complete_login`] ends it
/// when it succeeds; until then every caller gets the same one.
pub fn pending_login(store: StoreId) -> PendingLogin {
    let mut pending = PENDING_LOGINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    pending
        .entry(store)
        .or_insert_with(|| begin_login(store))
        .clone()
}

fn end_pending_login(store: StoreId) {
    PENDING_LOGINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&store);
}

/// Starts a new login. Front ends use [`pending_login`], which keeps one per store.
fn begin_login(store: StoreId) -> PendingLogin {
    match store {
        StoreId::Epic => epic::begin_login(),
        StoreId::Gog => gog::begin_login(),
        StoreId::Amazon => amazon::begin_login(),
        StoreId::Ea => ea::begin_login(),
        StoreId::BattleNet => battlenet::begin_login(),
        StoreId::Humble => humble::begin_login(),
        StoreId::Itch => itch::begin_login(),
    }
}

/// Finishes a login and stores the session. Returns the account name when the store has one.
pub async fn complete_login(
    pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Option<String>> {
    complete_login_inner(pending, capture)
        .await
        .map_err(redacted)
}

async fn complete_login_inner(
    pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Option<String>> {
    let store = pending.store;
    let client = http::client()?;
    let session = match store {
        StoreId::Epic => epic::complete_login(&client, pending, capture).await,
        StoreId::Gog => gog::complete_login(&client, pending, capture).await,
        StoreId::Amazon => amazon::complete_login(&client, pending, capture).await,
        StoreId::Ea => ea::complete_login(&client, pending, capture).await,
        StoreId::BattleNet => battlenet::complete_login(&client, pending, capture).await,
        StoreId::Humble => humble::complete_login(&client, pending, capture).await,
        StoreId::Itch => itch::complete_login(&client, pending, capture).await,
    }?;
    save_new_login(store, &session)?;
    end_pending_login(store);
    record_failure(store, None);
    Ok(session.account_name)
}

fn save_new_login(store: StoreId, session: &Session) -> eyre::Result<()> {
    let _lock = storage::lock_account(store)?;
    save_session(store, session)?;
    // The cached library may be another account's: fetch it again on next use.
    if let Some(mut library) = cached_library(store) {
        library.fetched_at = 0;
        save_library(store, &library)?;
    }
    Ok(())
}

/// The error as one message (with its causes) and without secrets, keeping whether the store
/// rejected the login.
fn redacted(err: eyre::Report) -> eyre::Report {
    let message = http::redact(&format!("{err:#}"));
    if http::is_login_rejected(&err) {
        http::login_rejected(message)
    } else if is_account_changed(&err) {
        eyre::Report::new(AccountChanged(message))
    } else {
        eyre::Report::msg(message)
    }
}

/// The account was signed out of, or signed in again, while a refresh was running. The
/// refresh's results belong to the old login and were dropped.
#[derive(Debug)]
struct AccountChanged(String);

impl std::fmt::Display for AccountChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AccountChanged {}

fn is_account_changed(err: &eyre::Report) -> bool {
    err.chain().any(|cause| cause.is::<AccountChanged>())
}

/// One lock per store: two refreshes of the same account at once would both use (and
/// rotate) the same refresh token and both write the same files.
fn refresh_lock(store: StoreId) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<BTreeMap<StoreId, Arc<tokio::sync::Mutex<()>>>> = OnceLock::new();
    LOCKS
        .get_or_init(|| {
            StoreId::ALL
                .into_iter()
                .map(|store| (store, Arc::new(tokio::sync::Mutex::new(()))))
                .collect()
        })
        .get(&store)
        .cloned()
        .unwrap_or_default()
}

/// The last failed refresh of a store, so listing games does not retry an unreachable store
/// on every call, and front ends can say what went wrong.
#[derive(Clone, Debug)]
struct Failure {
    /// Unix seconds.
    at: u64,
    message: String,
    needs_sign_in: bool,
}

static LAST_FAILURE: Mutex<BTreeMap<StoreId, Failure>> = Mutex::new(BTreeMap::new());

/// How long [`refresh_if_stale`] waits after a failed refresh before it tries again.
pub const RETRY_AFTER_FAILURE_SECS: u64 = 15 * 60;

/// Records the outcome of a refresh: `None` clears the failure.
fn record_failure(store: StoreId, error: Option<&eyre::Report>) {
    let mut failures = LAST_FAILURE.lock().unwrap_or_else(PoisonError::into_inner);
    match error {
        Some(err) => {
            let message = err.to_string();
            failures.insert(
                store,
                Failure {
                    at: now_unix(),
                    needs_sign_in: http::is_login_rejected(err)
                        || message.to_lowercase().contains("sign in again"),
                    message,
                },
            );
        }
        None => {
            failures.remove(&store);
        }
    }
}

fn last_failure(store: StoreId) -> Option<Failure> {
    LAST_FAILURE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&store)
        .cloned()
}

fn failed_recently(store: StoreId) -> bool {
    last_failure(store)
        .is_some_and(|failure| now_unix().saturating_sub(failure.at) < RETRY_AFTER_FAILURE_SECS)
}

/// Runs a refresh and records how it went. A refresh dropped because the account changed is
/// not a failure of the new login.
async fn refresh_and_record(store: StoreId) -> eyre::Result<OwnedLibrary> {
    let result = refresh_library_locked(store).await.map_err(redacted);
    match &result {
        Ok(_) => record_failure(store, None),
        Err(err) if is_account_changed(err) => {}
        Err(err) => record_failure(store, Some(err)),
    }
    result
}

/// The cached library, or `None` when there is none or it can't be read (a damaged file is
/// replaced by the next refresh instead of breaking the account).
pub fn cached_library(store: StoreId) -> Option<OwnedLibrary> {
    match load_library(store) {
        Ok(library) => library,
        Err(err) => {
            eprintln!(
                "The saved {} library is unreadable, it will be fetched again: {err}",
                store.display_name()
            );
            None
        }
    }
}

/// Fetches the owned games with the stored session, then stores the refreshed session and
/// the library. On error the previous library stays, so a network problem never empties it.
pub async fn refresh_library(store: StoreId) -> eyre::Result<OwnedLibrary> {
    let lock = refresh_lock(store);
    let _guard = lock.lock().await;
    refresh_and_record(store).await
}

/// The library for listing games: the cached one, fetched again first when it is older than
/// `max_age_secs`. A failed fetch falls back to the cache and is not retried for
/// [`RETRY_AFTER_FAILURE_SECS`]. Concurrent callers wait for one fetch instead of each
/// making their own.
pub async fn refresh_if_stale(store: StoreId, max_age_secs: u64) -> eyre::Result<OwnedLibrary> {
    let cached = cached_library(store);
    if !is_stale(cached.as_ref(), max_age_secs) || (cached.is_some() && failed_recently(store)) {
        return cached.ok_or_else(|| eyre::format_err!("No library fetched yet"));
    }
    let lock = refresh_lock(store);
    let _guard = lock.lock().await;
    // Another caller may have fetched it while this one waited.
    let cached = cached_library(store);
    if !is_stale(cached.as_ref(), max_age_secs) {
        return cached.ok_or_else(|| eyre::format_err!("No library fetched yet"));
    }
    if failed_recently(store) {
        return cached.ok_or_else(|| {
            eyre::format_err!(
                "Could not fetch the {} library a moment ago; it is tried again later",
                store.display_name()
            )
        });
    }
    match (refresh_and_record(store).await, cached) {
        (Ok(library), _) => Ok(library),
        (Err(err), _) if is_account_changed(&err) => Err(err),
        (Err(err), Some(cached)) => {
            eprintln!(
                "Could not refresh the {} library, using the one from before: {err}",
                store.display_name()
            );
            Ok(cached)
        }
        (Err(err), None) => Err(err),
    }
}

async fn refresh_library_locked(store: StoreId) -> eyre::Result<OwnedLibrary> {
    let started_with = load_session(store)?
        .ok_or_else(|| eyre::format_err!("Not signed in to {}", store.display_name()))?;
    let mut session = started_with.clone();
    let client = http::client()?;
    let result = match store {
        StoreId::Epic => epic::fetch_owned(&client, &mut session).await,
        StoreId::Gog => gog::fetch_owned(&client, &mut session).await,
        StoreId::Amazon => amazon::fetch_owned(&client, &mut session).await,
        StoreId::Ea => ea::fetch_owned(&client, &mut session).await,
        StoreId::BattleNet => battlenet::fetch_owned(&client, &mut session).await,
        StoreId::Humble => humble::fetch_owned(&client, &mut session).await,
        StoreId::Itch => itch::fetch_owned(&client, &mut session).await,
    };
    commit_refresh(store, &started_with, &session, result)
}

/// Saves what a refresh produced: the session (tokens may have been renewed even when listing
/// the games failed) and the library.
fn commit_refresh(
    store: StoreId,
    started_with: &Session,
    session: &Session,
    result: eyre::Result<Vec<OwnedGame>>,
) -> eyre::Result<OwnedLibrary> {
    let _lock = storage::lock_account(store)?;
    // Signing out (or in again) while the games were being fetched wins: saving now would
    // bring back the old login.
    if load_session(store)?.as_ref() != Some(started_with) {
        return Err(eyre::Report::new(AccountChanged(format!(
            "The {} account changed while its games were being fetched; that list was dropped.",
            store.display_name()
        ))));
    }
    if session != started_with {
        save_session(store, session)?;
    }
    let mut games = result?;
    games.retain(|g| !g.id.is_empty() && !g.title.trim().is_empty());
    games.sort_by(|a, b| a.id.cmp(&b.id));
    games.dedup_by(|a, b| a.id == b.id);
    if games.is_empty() {
        if let Some(before) = cached_library(store).filter(|l| !l.games.is_empty()) {
            return Err(eyre::format_err!(
                "{} listed no games, but it listed {} before. The earlier list is kept, \
                 because an empty answer is more likely a store problem than an empty \
                 account. If the account really has no games, sign out and in again.",
                store.display_name(),
                before.games.len()
            ));
        }
    }
    let library = OwnedLibrary {
        account_name: session.account_name.clone(),
        fetched_at: now_unix(),
        games,
    };
    save_library(store, &library)?;
    Ok(library)
}

/// Signs out. Where the store offers a way, it is told first that BoilR's login is no longer
/// needed: Amazon removes BoilR from the account's devices and Epic ends the session. The
/// other stores keep the login valid until it expires. Then the session and the cached
/// library are removed, also when the store could not be told; that case returns a warning.
pub async fn sign_out(store: StoreId) -> eyre::Result<Option<String>> {
    let warning = match load_session(store).ok().flatten() {
        Some(session) => end_remote_session(store, &session).await.err().map(|err| {
            format!(
                "Signed out, but {} may still list BoilR's login: {}",
                store.display_name(),
                redacted(err)
            )
        }),
        None => None,
    };
    delete_account(store)?;
    Ok(warning)
}

async fn end_remote_session(store: StoreId, session: &Session) -> eyre::Result<()> {
    let client = http::client()?;
    match store {
        StoreId::Amazon => amazon::sign_out(&client, session).await,
        StoreId::Epic => epic::sign_out(&client, session).await,
        StoreId::Gog | StoreId::Ea | StoreId::BattleNet | StoreId::Humble | StoreId::Itch => Ok(()),
    }
}

/// Removes the session and the cached library, without telling the store; see [`sign_out`].
pub fn delete_account(store: StoreId) -> eyre::Result<()> {
    {
        let _lock = storage::lock_account(store)?;
        storage::delete_account(store)?;
    }
    end_pending_login(store);
    record_failure(store, None);
    Ok(())
}

/// Whether a library is older than `max_age_secs` (or was never fetched).
pub fn is_stale(library: Option<&OwnedLibrary>, max_age_secs: u64) -> bool {
    match library {
        Some(library) => now_unix().saturating_sub(library.fetched_at) > max_age_secs,
        None => true,
    }
}

/// The state of one store account, for display.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountStatus {
    /// Written as the store's [`StoreId::key`] (`battlenet`), the same as `key`, so front
    /// ends can use either.
    #[serde(with = "store_key")]
    pub store: StoreId,
    pub key: String,
    pub name: String,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub game_count: Option<usize>,
    pub fetched_at: Option<u64>,
    pub manual: ManualLogin,
    /// Whether the built-in login window can sign in to this store.
    pub webview_login: bool,
    /// Why the last refresh failed, while it has not succeeded since (this run of the app).
    pub last_error: Option<String>,
    /// The last refresh failed because the store no longer accepts the saved login.
    pub needs_sign_in: bool,
}

/// A [`StoreId`] as its key. `StoreId`'s own serde form (`battle_net`) stays as it is: saved
/// libraries use it.
mod store_key {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    use crate::StoreId;

    pub fn serialize<S: Serializer>(store: &StoreId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(store.key())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<StoreId, D::Error> {
        let key = String::deserialize(deserializer)?;
        StoreId::from_key(&key).ok_or_else(|| D::Error::custom(format!("unknown store {key}")))
    }
}

/// Reads the session and the cached library from disk; front ends call it after a change,
/// not on every repaint.
pub fn account_status(store: StoreId) -> AccountStatus {
    let session = load_session(store).ok().flatten();
    let library = load_library(store).ok().flatten();
    let pending = pending_login(store);
    let failure = last_failure(store).filter(|_| session.is_some());
    AccountStatus {
        store,
        key: store.key().to_string(),
        name: store.display_name().to_string(),
        signed_in: session.is_some(),
        account_name: session
            .and_then(|s| s.account_name)
            .or_else(|| library.as_ref().and_then(|l| l.account_name.clone())),
        game_count: library.as_ref().map(|l| l.games.len()),
        fetched_at: library.map(|l| l.fetched_at),
        manual: pending.manual,
        webview_login: !pending.done_url_prefixes.is_empty(),
        needs_sign_in: failure.as_ref().is_some_and(|f| f.needs_sign_in),
        last_error: failure.map(|f| f.message),
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_in_keeps_its_state_until_it_ends() {
        // Amazon's sign-in link carries a PKCE challenge: the link the user opened and the
        // login that finishes it must share the verifier.
        let opened = pending_login(StoreId::Amazon);
        let finished = pending_login(StoreId::Amazon);
        assert_eq!(opened.start_url, finished.start_url);
        assert_eq!(opened.state, finished.state);

        end_pending_login(StoreId::Amazon);
        assert_ne!(pending_login(StoreId::Amazon).state, opened.state);
    }

    #[test]
    fn recent_failures_hold_off_retries() {
        record_failure(
            StoreId::Humble,
            Some(&http::login_rejected("401 from https://x. Sign in again.")),
        );
        assert!(failed_recently(StoreId::Humble));
        assert!(last_failure(StoreId::Humble).is_some_and(|f| f.needs_sign_in));
        record_failure(StoreId::Humble, None);
        assert!(!failed_recently(StoreId::Humble));
    }

    #[test]
    fn account_status_names_the_store_by_its_key() -> eyre::Result<()> {
        let status = serde_json::to_value(account_status(StoreId::BattleNet))?;
        assert_eq!(status.get("store"), Some(&serde_json::json!("battlenet")));
        assert_eq!(status.get("key"), Some(&serde_json::json!("battlenet")));
        let back: AccountStatus = serde_json::from_value(status)?;
        assert_eq!(back.store, StoreId::BattleNet);
        Ok(())
    }

    #[test]
    fn redacted_errors_keep_their_kind_and_lose_secrets() {
        let err = http::login_rejected("401 from https://a.com/x?refresh_token=SECRET")
            .wrap_err("Could not load the library");
        let clean = redacted(err);
        assert!(http::is_login_rejected(&clean));
        let message = clean.to_string();
        assert!(message.contains("Could not load the library"), "{message}");
        assert!(message.contains("https://a.com/x"), "{message}");
        assert!(!message.contains("SECRET"), "{message}");
    }

    fn session(token: &str) -> Session {
        Session {
            account_name: Some("player".into()),
            data: serde_json::json!({ "token": token }),
        }
    }

    fn library(count: usize) -> OwnedLibrary {
        OwnedLibrary {
            account_name: Some("player".into()),
            fetched_at: 1,
            games: (0..count)
                .map(|i| OwnedGame::new(StoreId::Itch, i.to_string(), "Game"))
                .collect(),
        }
    }

    fn games(count: usize) -> eyre::Result<Vec<OwnedGame>> {
        Ok(library(count).games)
    }

    // Each test uses its own store: tests run at the same time and share the test folder.
    #[test]
    fn a_refresh_never_saves_over_a_sign_out() -> eyre::Result<()> {
        let store = StoreId::Itch;
        let started_with = session("old");
        save_session(store, &started_with)?;
        delete_account(store)?;

        let result = commit_refresh(store, &started_with, &session("renewed"), games(2));
        assert!(result.is_err_and(|e| is_account_changed(&e)));
        assert_eq!(load_session(store)?, None);
        assert!(cached_library(store).is_none());
        Ok(())
    }

    #[test]
    fn a_refresh_never_saves_over_a_new_sign_in() -> eyre::Result<()> {
        let store = StoreId::BattleNet;
        let started_with = session("old");
        save_session(store, &started_with)?;
        save_new_login(store, &session("new"))?;

        let result = commit_refresh(store, &started_with, &session("renewed"), games(2));
        assert!(result.is_err_and(|e| is_account_changed(&e)));
        assert_eq!(load_session(store)?, Some(session("new")));
        delete_account(store)?;
        Ok(())
    }

    #[test]
    fn a_refresh_saves_renewed_tokens_even_when_listing_fails() -> eyre::Result<()> {
        let store = StoreId::Amazon;
        let started_with = session("old");
        save_session(store, &started_with)?;

        let failed = commit_refresh(
            store,
            &started_with,
            &session("renewed"),
            Err(eyre::format_err!("network down")),
        );
        assert!(failed.is_err());
        assert_eq!(load_session(store)?, Some(session("renewed")));

        let renewed = session("renewed");
        let saved = commit_refresh(store, &renewed, &renewed, games(2))?;
        assert_eq!(saved.games.len(), 2);
        assert_eq!(cached_library(store).map(|l| l.games.len()), Some(2));
        delete_account(store)?;
        Ok(())
    }

    #[test]
    fn an_empty_answer_does_not_replace_a_library() -> eyre::Result<()> {
        let store = StoreId::Humble;
        let current = session("t");
        save_session(store, &current)?;
        save_library(store, &library(3))?;

        assert!(commit_refresh(store, &current, &current, games(0)).is_err());
        assert_eq!(cached_library(store).map(|l| l.games.len()), Some(3));

        // After signing out and in again there is no earlier list to protect.
        delete_account(store)?;
        save_session(store, &current)?;
        assert!(commit_refresh(store, &current, &current, games(0))?
            .games
            .is_empty());
        delete_account(store)?;
        Ok(())
    }

    #[test]
    fn signing_in_again_makes_the_library_stale() -> eyre::Result<()> {
        let store = StoreId::Epic;
        save_library(
            store,
            &OwnedLibrary {
                fetched_at: now_unix(),
                ..library(3)
            },
        )?;
        save_new_login(store, &session("new"))?;
        assert!(is_stale(cached_library(store).as_ref(), 60));
        assert_eq!(cached_library(store).map(|l| l.games.len()), Some(3));
        delete_account(store)?;
        Ok(())
    }
}
