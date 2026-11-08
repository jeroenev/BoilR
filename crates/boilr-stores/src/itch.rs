//! itch.io: every game the account holds a download key for (purchases, claims, bundles,
//! gifts), installed or not.
//!
//! Uses itch's server-side API (https://itch.io/docs/api/serverside) with an API key the user
//! creates on their account settings page: `GET https://api.itch.io/profile` for the account
//! and `GET https://api.itch.io/profile/owned-keys?page=N` for the keys, each with an embedded
//! game. Playnite's ItchioLibrary reads the same data through the itch app's butler
//! (`Fetch.ProfileOwnedKeys`); this needs no itch app.
//!
//! Like Playnite's default settings, only items classified `game` or `tool` are listed;
//! assets, soundtracks, books, comics, mods, physical games and "other" are skipped. Free
//! games the user only added to a collection have no key and are not listed (Playnite's
//! default too).
//!
//! `OwnedGame.id` is the itch game id as a decimal string. `extra` keys:
//! - `url`: the game's page on itch.io.
//! - `classification`: itch's classification (`game` or `tool`).
//! - `platforms`: comma-separated platforms with downloads (`windows`, `linux`, `mac`,
//!   `android`), plus `web` for games played in the browser.

use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{http, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId};

const API_KEYS_URL: &str = "https://itch.io/user/settings/api-keys";
const PROFILE_URL: &str = "https://api.itch.io/profile";
const OWNED_KEYS_URL: &str = "https://api.itch.io/profile/owned-keys";
/// The documented maximum; the default is 50.
const KEYS_PER_PAGE: &str = "500";
/// A stop for a server that never returns an empty page (that would be 500 000 keys).
const MAX_PAGES: u32 = 1000;
/// Playnite's default `ImportGameClassification`.
const LISTED_CLASSIFICATIONS: [&str; 2] = ["game", "tool"];

/// What BoilR stores for an itch.io account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ItchSession {
    api_key: String,
    #[serde(default)]
    user_id: Option<u64>,
}

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::Itch,
        start_url: API_KEYS_URL.to_string(),
        // No login window: itch.io sign-in goes through an API key.
        done_url_prefixes: vec![],
        cookie_urls: vec![],
        manual: ManualLogin::Paste {
            url: API_KEYS_URL.to_string(),
            instructions: "Sign in to itch.io in your browser and open your API keys page \
                           (Settings > API keys). Click \"Generate new API key\", then \
                           \"View\" next to the new key, and paste the key here. You can \
                           revoke it on the same page at any time."
                .to_string(),
            placeholder: "itch.io API key".to_string(),
        },
        user_agent: None,
        state: serde_json::Value::Null,
    }
}

pub async fn complete_login(
    client: &Client,
    _pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Session> {
    let api_key = capture
        .pasted
        .as_deref()
        .and_then(parse_pasted_key)
        .ok_or_else(|| {
            eyre::format_err!(
                "That is not an itch.io API key. Copy the key from {API_KEYS_URL} and paste it."
            )
        })?;
    let user = fetch_profile(client, &api_key).await?;
    Ok(Session {
        account_name: user.account_name(),
        data: serde_json::to_value(ItchSession {
            api_key,
            user_id: user.id,
        })?,
    })
}

pub async fn fetch_owned(client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let data: ItchSession = serde_json::from_value(session.data.clone()).map_err(|err| {
        eyre::format_err!("The saved itch.io login is unreadable ({err}). Sign in again.")
    })?;
    let mut keys = Vec::new();
    for page in 1..=MAX_PAGES {
        let page = page.to_string();
        let request = client
            .get(OWNED_KEYS_URL)
            .query(&[("page", page.as_str()), ("per_page", KEYS_PER_PAGE)])
            .bearer_auth(&data.api_key);
        let text = http::send(request).await?.text().await?;
        let page_keys = parse_owned_keys(&text)?;
        // Short pages do not mean the end (the docs say so); only an empty page does.
        if page_keys.is_empty() {
            break;
        }
        keys.extend(page_keys);
    }
    Ok(owned_games(keys))
}

async fn fetch_profile(client: &Client, api_key: &str) -> eyre::Result<User> {
    let text = http::send(client.get(PROFILE_URL).bearer_auth(api_key))
        .await?
        .text()
        .await?;
    parse_profile(&text)
}

/// Trims the pasted key. API keys are a single token of letters and digits.
fn parse_pasted_key(pasted: &str) -> Option<String> {
    let key = pasted.trim();
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| key.to_string())
}

#[derive(Debug, Default, Deserialize)]
struct User {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

impl User {
    fn account_name(&self) -> Option<String> {
        [&self.username, &self.display_name]
            .into_iter()
            .flatten()
            .map(|n| n.trim())
            .find(|n| !n.is_empty())
            .map(str::to_string)
    }
}

#[derive(Deserialize)]
struct ProfileResponse {
    user: User,
}

#[derive(Deserialize)]
struct OwnedKeysResponse {
    #[serde(default)]
    owned_keys: Option<Vec<OwnedKey>>,
}

#[derive(Debug, Deserialize)]
struct OwnedKey {
    #[serde(default)]
    game_id: Option<u64>,
    #[serde(default)]
    game: Option<Game>,
}

#[derive(Debug, Default, Deserialize)]
struct Game {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    /// `default`, `flash`, `unity`, `java` or `html`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    classification: Option<String>,
    /// `p_windows`, `p_linux`, `p_osx`, `p_android`, `can_be_bought`, `has_demo`, ...
    #[serde(default)]
    traits: Option<Vec<String>>,
}

fn parse_profile(text: &str) -> eyre::Result<User> {
    let response: ProfileResponse = parse_api(text, "profile")?;
    Ok(response.user)
}

fn parse_owned_keys(text: &str) -> eyre::Result<Vec<OwnedKey>> {
    let response: OwnedKeysResponse = parse_api(text, "owned games")?;
    Ok(response.owned_keys.unwrap_or_default())
}

/// Decodes an api.itch.io response. Errors come as `{"errors": ["invalid key"]}`, which may
/// arrive with a success status.
fn parse_api<T: DeserializeOwned>(text: &str, what: &str) -> eyre::Result<T> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|err| {
        eyre::format_err!(
            "Unexpected {what} response from itch.io: {err}. Body starts with: {}",
            http::snippet(text)
        )
    })?;
    if let Some(errors) = value.get("errors").and_then(|e| e.as_array()) {
        let messages: Vec<&str> = errors.iter().filter_map(|e| e.as_str()).collect();
        let joined = messages.join("; ");
        if messages
            .iter()
            .any(|m| m.contains("key") || m.contains("auth") || m.contains("scope"))
        {
            return Err(eyre::format_err!(
                "itch.io rejected the API key ({joined}). Create a new key at {API_KEYS_URL} and sign in again."
            ));
        }
        return Err(eyre::format_err!("itch.io returned an error: {joined}"));
    }
    serde_json::from_value(value).map_err(|err| {
        eyre::format_err!(
            "Unexpected {what} response from itch.io: {err}. Body starts with: {}",
            http::snippet(text)
        )
    })
}

/// One `OwnedGame` per listed game, in key order, without duplicates.
fn owned_games(keys: Vec<OwnedKey>) -> Vec<OwnedGame> {
    let mut seen = std::collections::BTreeSet::new();
    let mut games = Vec::new();
    for key in keys {
        let Some(game) = key.game else {
            continue;
        };
        let Some(id) = game.id.or(key.game_id) else {
            continue;
        };
        let title = game.title.as_deref().map(clean_title).unwrap_or_default();
        if title.is_empty() || !is_listed(game.classification.as_deref()) || !seen.insert(id) {
            continue;
        }
        let mut owned = OwnedGame::new(StoreId::Itch, id.to_string(), title)
            .with_extra(
                "classification",
                game.classification.as_deref().unwrap_or("game"),
            )
            .with_extra("platforms", platforms(&game).join(","));
        if let Some(url) = game.url.as_deref().filter(|u| !u.is_empty()) {
            owned = owned.with_extra("url", url);
        }
        games.push(owned);
    }
    games
}

/// A missing classification is taken as a game; unknown ones are skipped.
fn is_listed(classification: Option<&str>) -> bool {
    classification.is_none_or(|c| LISTED_CLASSIFICATIONS.contains(&c))
}

fn platforms(game: &Game) -> Vec<&'static str> {
    let traits = game.traits.as_deref().unwrap_or_default();
    let has = |t: &str| traits.iter().any(|x| x == t);
    let mut platforms: Vec<&'static str> = [
        ("p_windows", "windows"),
        ("p_linux", "linux"),
        ("p_osx", "mac"),
        ("p_android", "android"),
    ]
    .into_iter()
    .filter(|(t, _)| has(t))
    .map(|(_, p)| p)
    .collect();
    if game.kind.as_deref() == Some("html") {
        platforms.push("web");
    }
    platforms
}

/// Removes trademark signs, as Playnite's `RemoveTrademarks` does.
fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '™' | '®' | '©'))
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = include_str!("../testdata/itch/profile.json");
    const OWNED_KEYS: &str = include_str!("../testdata/itch/owned_keys_page1.json");
    const OWNED_KEYS_EMPTY: &str = include_str!("../testdata/itch/owned_keys_empty.json");
    const INVALID_KEY: &str = include_str!("../testdata/itch/invalid_key.json");

    #[test]
    fn has_no_login_window() {
        let pending = begin_login();
        assert!(pending.done_url_prefixes.is_empty());
        assert!(matches!(pending.manual, ManualLogin::Paste { .. }));
    }

    #[test]
    fn reads_profile() -> eyre::Result<()> {
        let user = parse_profile(PROFILE)?;
        assert_eq!(user.id, Some(3423));
        assert_eq!(user.account_name(), Some("leafo".to_string()));
        Ok(())
    }

    #[test]
    fn invalid_key_is_a_readable_error() {
        let err = parse_profile(INVALID_KEY).err().map(|e| e.to_string());
        assert!(err.is_some_and(|e| e.contains("rejected the API key")));
    }

    #[test]
    fn lists_games_and_tools_and_skips_other_classifications() -> eyre::Result<()> {
        let games = owned_games(parse_owned_keys(OWNED_KEYS)?);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        // 999 is a second key for 284880, 555 an asset pack, 556 a soundtrack, 557 has no game.
        assert_eq!(ids, vec!["284880", "123456", "777"]);

        let pikuniku = games.first();
        assert_eq!(pikuniku.map(|g| g.title.as_str()), Some("Pikuniku"));
        assert_eq!(
            pikuniku.and_then(|g| g.extra("url")),
            Some("https://sectordub.itch.io/pikuniku")
        );
        assert_eq!(
            pikuniku.and_then(|g| g.extra("classification")),
            Some("game")
        );
        assert_eq!(
            pikuniku.and_then(|g| g.extra("platforms")),
            Some("windows,linux,mac")
        );

        let tool = games.get(1);
        assert_eq!(tool.and_then(|g| g.extra("classification")), Some("tool"));
        assert_eq!(tool.map(|g| g.title.as_str()), Some("Pixel Tool"));

        let web = games.get(2);
        assert_eq!(web.and_then(|g| g.extra("platforms")), Some("web"));
        Ok(())
    }

    #[test]
    fn empty_page_ends_paging() -> eyre::Result<()> {
        assert!(parse_owned_keys(OWNED_KEYS_EMPTY)?.is_empty());
        assert!(parse_owned_keys("{\"page\": 3, \"per_page\": 50}")?.is_empty());
        Ok(())
    }

    #[test]
    fn reads_pasted_key() {
        assert_eq!(parse_pasted_key("  AbC123xyz \n"), Some("AbC123xyz".into()));
        assert_eq!(parse_pasted_key(""), None);
        assert_eq!(parse_pasted_key("not a key"), None);
    }
}
