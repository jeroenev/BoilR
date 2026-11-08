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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use storage::{delete_account, load_library, load_session, save_library, save_session};

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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
    /// Store-private state carried from [`begin_login`] to [`complete_login`] (PKCE
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

/// Starts a login.
pub fn begin_login(store: StoreId) -> PendingLogin {
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
    let client = http::client()?;
    let session = match pending.store {
        StoreId::Epic => epic::complete_login(&client, pending, capture).await,
        StoreId::Gog => gog::complete_login(&client, pending, capture).await,
        StoreId::Amazon => amazon::complete_login(&client, pending, capture).await,
        StoreId::Ea => ea::complete_login(&client, pending, capture).await,
        StoreId::BattleNet => battlenet::complete_login(&client, pending, capture).await,
        StoreId::Humble => humble::complete_login(&client, pending, capture).await,
        StoreId::Itch => itch::complete_login(&client, pending, capture).await,
    }?;
    save_session(pending.store, &session)?;
    Ok(session.account_name)
}

/// Fetches the owned games with the stored session, then stores the refreshed session and
/// the library. On error the previous library stays, so a network problem never empties it.
pub async fn refresh_library(store: StoreId) -> eyre::Result<OwnedLibrary> {
    let mut session = load_session(store)?
        .ok_or_else(|| eyre::format_err!("Not signed in to {}", store.display_name()))?;
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
    // Tokens may have been refreshed even when listing the games failed.
    save_session(store, &session)?;
    let mut games = result?;
    games.retain(|g| !g.id.is_empty() && !g.title.trim().is_empty());
    games.sort_by(|a, b| a.id.cmp(&b.id));
    games.dedup_by(|a, b| a.id == b.id);
    let library = OwnedLibrary {
        account_name: session.account_name.clone(),
        fetched_at: now_unix(),
        games,
    };
    save_library(store, &library)?;
    Ok(library)
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
}

pub fn account_status(store: StoreId) -> AccountStatus {
    let session = load_session(store).ok().flatten();
    let library = load_library(store).ok().flatten();
    let pending = begin_login(store);
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
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
