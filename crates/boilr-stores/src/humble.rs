//! Humble Bundle: the DRM-free games in the account's purchases (bundles, store orders,
//! claimed Choice games), installed or not.
//!
//! Modelled on Playnite's HumbleLibrary (`source/Libraries/HumbleLibrary`): the session is
//! the `_simpleauth_sess` cookie of humblebundle.com; the order keys come from
//! `api/v1/user/order` (or, as Playnite does, from the JSON embedded in the library page),
//! and the orders themselves from `api/v1/orders?all_tpkds=true&gamekeys=...` in batches of 40.
//!
//! A subproduct counts as a game when it has a Windows, Linux or Mac download. Products that
//! are only keys for another store (Steam, GOG, ...) have no download and are skipped; like
//! Playnite's default (`IgnoreThirdPartyStoreGames`), a DRM-free game is also skipped when the
//! account got a third-party key for it, because it shows up in that store's library.
//! Humble Trove / Games Collection is not listed (Playnite's default too).
//!
//! `OwnedGame.id` is the subproduct's `machine_name`. `extra` keys:
//! - `gamekey`: the order the game came with.
//! - `url`: the page where the user downloads it (`/downloads?key=<gamekey>`).
//! - `platforms`: comma-separated download platforms (`windows`, `linux`, `mac`).

use std::collections::{BTreeMap, BTreeSet};

use reqwest::{header::HeaderMap, Client};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{http, Cookie, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId};

const LOGIN_URL: &str = "https://www.humblebundle.com/login?goto=%2Fhome%2Flibrary";
const LIBRARY_URL: &str = "https://www.humblebundle.com/home/library";
/// After signing in, Humble sends the browser to `goto`, the library page. Only that page
/// ends the login, not any `/home` page the user might reach on the way.
const DONE_URL_PREFIX: &str = "https://www.humblebundle.com/home/library";
const COOKIE_URL: &str = "https://www.humblebundle.com/";
const COOKIE_HOST: &str = "www.humblebundle.com";
const ORDER_LIST_URL: &str = "https://www.humblebundle.com/api/v1/user/order";
const ORDERS_URL: &str = "https://www.humblebundle.com/api/v1/orders";
const SESSION_COOKIE: &str = "_simpleauth_sess";
/// Playnite asks for 40 orders per request.
const ORDERS_PER_REQUEST: usize = 40;
/// Download platforms that make a subproduct a game BoilR can install and launch.
const GAME_PLATFORMS: [&str; 3] = ["windows", "linux", "mac"];

/// What BoilR stores for a Humble account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HumbleSession {
    /// Value of the `_simpleauth_sess` cookie, as the browser holds it (often in quotes).
    simpleauth_sess: String,
}

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::Humble,
        start_url: LOGIN_URL.to_string(),
        done_url_prefixes: vec![DONE_URL_PREFIX.to_string()],
        cookie_urls: vec![COOKIE_URL.to_string()],
        manual: ManualLogin::Paste {
            url: LOGIN_URL.to_string(),
            instructions: format!(
                "Sign in to humblebundle.com in your browser. Then open the developer tools \
                 (F12), go to Storage (Firefox) or Application (Chrome) > Cookies > \
                 https://www.humblebundle.com, and copy the value of the cookie named \
                 {SESSION_COOKIE}. Paste it here. It is your login, so do not share it."
            ),
            placeholder: format!("{SESSION_COOKIE} cookie value"),
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
    let mut session = HumbleSession {
        simpleauth_sess: session_cookie_from_capture(capture)?,
    };
    // Proves the cookie works before it is stored.
    fetch_gamekeys(client, &mut session).await?;
    Ok(Session {
        account_name: None,
        data: serde_json::to_value(&session)?,
    })
}

pub async fn fetch_owned(client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut data: HumbleSession = serde_json::from_value(session.data.clone()).map_err(|err| {
        eyre::format_err!("The saved Humble Bundle login is unreadable ({err}). Sign in again.")
    })?;
    let gamekeys = fetch_gamekeys(client, &mut data).await;
    // Humble may hand out a renewed cookie; keep it even when a later step fails.
    session.data = serde_json::to_value(&data)?;
    let gamekeys = gamekeys?;
    let orders = fetch_orders(client, &mut data, &gamekeys).await;
    session.data = serde_json::to_value(&data)?;
    Ok(owned_games(&orders?))
}

/// The order keys of the account: from `api/v1/user/order`, or from the library page when
/// that endpoint does not answer as expected.
async fn fetch_gamekeys(client: &Client, session: &mut HumbleSession) -> eyre::Result<Vec<String>> {
    let api_error = match get_text(client, ORDER_LIST_URL, &[("ajax", "true")], session).await {
        Ok(text) => match parse_gamekeys(&text) {
            Ok(keys) => return Ok(keys),
            Err(err) => err,
        },
        Err(err) => err,
    };
    let page = get_text(client, LIBRARY_URL, &[], session).await?;
    parse_library_page_gamekeys(&page).ok_or(api_error)
}

async fn fetch_orders(
    client: &Client,
    session: &mut HumbleSession,
    gamekeys: &[String],
) -> eyre::Result<Vec<Order>> {
    let mut orders = Vec::new();
    for batch in gamekeys.chunks(ORDERS_PER_REQUEST) {
        let mut query = vec![("all_tpkds", "true")];
        query.extend(batch.iter().map(|key| ("gamekeys", key.as_str())));
        let text = get_text(client, ORDERS_URL, &query, session).await?;
        orders.extend(parse_orders(&text)?);
    }
    Ok(orders)
}

/// GETs a humblebundle.com page with the session cookie and returns the body. Picks up a
/// renewed session cookie, and turns a bounce to the login page into a sign-in-again error.
async fn get_text(
    client: &Client,
    url: &str,
    query: &[(&str, &str)],
    session: &mut HumbleSession,
) -> eyre::Result<String> {
    let request = client
        .get(url)
        .query(query)
        .header(
            reqwest::header::COOKIE,
            format!("{SESSION_COOKIE}={}", session.simpleauth_sess),
        )
        .header("X-Requested-With", "XMLHttpRequest");
    let response = http::send(request).await?;
    if let Some(renewed) = renewed_session_cookie(response.headers()) {
        session.simpleauth_sess = renewed;
    }
    let final_url = response.url().to_string();
    let text = response.text().await?;
    if final_url.contains("/login") {
        return Err(not_signed_in());
    }
    Ok(text)
}

fn not_signed_in() -> eyre::Report {
    eyre::format_err!("Humble Bundle did not accept the saved login. Sign in again.")
}

/// The session cookie from a webview login, or the value the user pasted.
fn session_cookie_from_capture(capture: &LoginCapture) -> eyre::Result<String> {
    if let Some(value) = session_cookie_from_cookies(&capture.cookies) {
        return Ok(value);
    }
    if let Some(value) = capture.pasted.as_deref().and_then(parse_pasted_cookie) {
        return Ok(value);
    }
    if capture.pasted.is_some() {
        return Err(eyre::format_err!(
            "That is not a Humble Bundle session cookie. Paste the value of the {SESSION_COOKIE} cookie."
        ));
    }
    Err(eyre::format_err!(
        "The Humble Bundle login did not finish: no {SESSION_COOKIE} cookie was found. Try signing in again."
    ))
}

fn session_cookie_from_cookies(cookies: &[Cookie]) -> Option<String> {
    cookies
        .iter()
        .filter(|c| c.name == SESSION_COOKIE && http::domain_matches(&c.domain, COOKIE_HOST))
        .map(|c| c.value.trim())
        .find(|v| is_usable_cookie_value(v))
        .map(str::to_string)
}

/// Accepts the bare value, or `_simpleauth_sess=value` as copied from a cookie header.
fn parse_pasted_cookie(pasted: &str) -> Option<String> {
    let pasted = pasted.trim();
    let value = match pasted.split_once('=') {
        Some((name, rest)) if name.trim() == SESSION_COOKIE => rest,
        _ => pasted,
    };
    let value = value.split(';').next().unwrap_or_default().trim();
    is_usable_cookie_value(value).then(|| value.to_string())
}

fn is_usable_cookie_value(value: &str) -> bool {
    let unquoted = value.trim_matches('"');
    !unquoted.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A renewed `_simpleauth_sess` from `Set-Cookie` headers. Deletions (empty values) are ignored.
fn renewed_session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .rev()
        .filter_map(|h| h.to_str().ok())
        .find_map(|h| {
            let (name, rest) = h.split_once('=')?;
            if name.trim() != SESSION_COOKIE {
                return None;
            }
            let value = rest.split(';').next()?.trim();
            is_usable_cookie_value(value).then(|| value.to_string())
        })
}

#[derive(Deserialize)]
struct OrderListEntry {
    gamekey: Option<String>,
}

/// Parses `api/v1/user/order`: `[{"gamekey": "..."}, ...]`.
fn parse_gamekeys(text: &str) -> eyre::Result<Vec<String>> {
    if looks_like_html(text) {
        return Err(not_signed_in());
    }
    let entries: Vec<Option<OrderListEntry>> = serde_json::from_str(text).map_err(|err| {
        eyre::format_err!(
            "Unexpected order list from Humble Bundle: {err}. Body starts with: {}",
            http::snippet(text)
        )
    })?;
    Ok(entries
        .into_iter()
        .flatten()
        .filter_map(|e| e.gamekey)
        .filter(|k| !k.is_empty())
        .collect())
}

#[derive(Deserialize)]
struct UserHome {
    #[serde(default, deserialize_with = "null_default")]
    gamekeys: Vec<String>,
}

/// The order keys in the library page's `<script id="user-home-json-data">` (what Playnite
/// reads). `None` when the page has no such data, which means the user is not signed in.
fn parse_library_page_gamekeys(html: &str) -> Option<Vec<String>> {
    let marker = html.find("id=\"user-home-json-data\"")?;
    let after_marker = html.get(marker..)?;
    let start = after_marker.find('>')? + 1;
    let body = after_marker.get(start..)?;
    let end = body.find("</script>")?;
    let home: UserHome = serde_json::from_str(body.get(..end)?.trim()).ok()?;
    Some(home.gamekeys)
}

fn looks_like_html(text: &str) -> bool {
    text.trim_start().starts_with('<')
}

#[derive(Debug, Default, Deserialize)]
struct Order {
    #[serde(default)]
    gamekey: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    subproducts: Vec<SubProduct>,
    #[serde(default)]
    tpkd_dict: Option<TpkdDict>,
}

#[derive(Debug, Default, Deserialize)]
struct SubProduct {
    #[serde(default)]
    machine_name: Option<String>,
    #[serde(default)]
    human_name: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    downloads: Vec<Download>,
}

#[derive(Debug, Default, Deserialize)]
struct Download {
    #[serde(default)]
    platform: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TpkdDict {
    #[serde(default, deserialize_with = "null_default")]
    all_tpks: Vec<Tpk>,
}

/// A third-party key (Steam, GOG, Origin, ...) that came with an order.
#[derive(Debug, Default, Deserialize)]
struct Tpk {
    #[serde(default)]
    human_name: Option<String>,
}

/// Parses `api/v1/orders`: an object from order key to order. Some accounts get `null`
/// orders (Playnite filters them out too).
fn parse_orders(text: &str) -> eyre::Result<Vec<Order>> {
    if looks_like_html(text) {
        return Err(not_signed_in());
    }
    let orders: std::collections::BTreeMap<String, Option<Order>> = serde_json::from_str(text)
        .map_err(|err| {
            eyre::format_err!(
                "Unexpected orders from Humble Bundle: {err}. Body starts with: {}",
                http::snippet(text)
            )
        })?;
    Ok(orders.into_values().flatten().collect())
}

/// The games in the orders, following Playnite's `GetLibraryGames` with its default settings.
///
/// A title in several orders is listed once. Its id is the smallest `machine_name` among
/// them, not the one from whichever order comes first, so buying a game again in another
/// bundle does not give it a new id (which would make a new Steam shortcut).
fn owned_games(orders: &[Order]) -> Vec<OwnedGame> {
    let mut by_title: BTreeMap<&str, OwnedGame> = BTreeMap::new();
    for order in orders {
        let Some(gamekey) = order.gamekey.as_deref().filter(|k| !k.is_empty()) else {
            continue;
        };
        // Like Playnite, only the keys of the same order hide a DRM-free game.
        let key_names: Vec<&str> = order
            .tpkd_dict
            .iter()
            .flat_map(|t| &t.all_tpks)
            .filter_map(|t| t.human_name.as_deref())
            .collect();
        for product in &order.subproducts {
            let (Some(machine_name), Some(human_name)) = (
                product.machine_name.as_deref(),
                product.human_name.as_deref(),
            ) else {
                continue;
            };
            if machine_name.is_empty() {
                continue;
            }
            let platforms: BTreeSet<&str> = product
                .downloads
                .iter()
                .filter_map(|d| d.platform.as_deref())
                .filter(|p| GAME_PLATFORMS.contains(p))
                .collect();
            if platforms.is_empty() || key_names.iter().any(|k| is_key_for(k, human_name)) {
                continue;
            }
            if by_title
                .get(human_name)
                .is_some_and(|listed| listed.id.as_str() <= machine_name)
            {
                continue;
            }
            let platforms: Vec<&str> = platforms.into_iter().collect();
            by_title.insert(
                human_name,
                OwnedGame::new(StoreId::Humble, machine_name, clean_title(human_name))
                    .with_extra("gamekey", gamekey)
                    .with_extra(
                        "url",
                        format!("https://www.humblebundle.com/downloads?key={gamekey}"),
                    )
                    .with_extra("platforms", platforms.join(",")),
            );
        }
    }
    by_title.into_values().collect()
}

/// Words that may follow a game's name in the name of its third-party key.
const KEY_NAME_WORDS: &[&str] = &[
    "key",
    "keys",
    "steam",
    "epic",
    "games",
    "store",
    "gog",
    "origin",
    "ea",
    "app",
    "uplay",
    "ubisoft",
    "connect",
    "battle",
    "net",
    "rockstar",
    "launcher",
    "microsoft",
    "xbox",
    "pc",
    "drm",
    "free",
];

/// Whether a third-party key's name is for the game `title`: the same name, or the name
/// followed only by a store or "key" ("FEZ Steam Key", "FEZ (Steam)", "FEZ - Epic Games
/// Key"), the patterns of Playnite's HumbleLibrary. A key for another game whose name merely
/// contains the title ("FEZ II Steam Key", "Limbonauts") does not count.
fn is_key_for(key_name: &str, title: &str) -> bool {
    let normalize = |name: &str| {
        clean_title(name)
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (key_name, title) = (normalize(key_name), normalize(title));
    if title.is_empty() {
        return false;
    }
    let Some(rest) = key_name.strip_prefix(&title) else {
        return false;
    };
    if rest.starts_with(char::is_alphanumeric) {
        return false;
    }
    rest.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .all(|word| KEY_NAME_WORDS.contains(&word))
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

/// Treats an explicit JSON `null` like a missing field.
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str = include_str!("../testdata/humble/orders.json");
    const ORDER_LIST: &str = include_str!("../testdata/humble/user_order.json");
    const LIBRARY_PAGE: &str = include_str!("../testdata/humble/library_page.html");

    fn ids(games: &[OwnedGame]) -> Vec<&str> {
        games.iter().map(|g| g.id.as_str()).collect()
    }

    #[test]
    fn login_start_page_is_not_a_done_page() {
        let pending = begin_login();
        assert!(!pending.done_url_prefixes.is_empty());
        for prefix in &pending.done_url_prefixes {
            assert!(!pending.start_url.starts_with(prefix.as_str()));
            assert!(
                !"https://www.humblebundle.com/login?goto=%2Fhome%2Flibrary&qs=x"
                    .starts_with(prefix.as_str())
            );
        }
        assert!(LIBRARY_URL.starts_with(DONE_URL_PREFIX));
        assert!(
            "https://www.humblebundle.com/home/library?hmb_source=navbar"
                .starts_with(DONE_URL_PREFIX)
        );
    }

    #[test]
    fn reads_gamekeys_from_order_list() -> eyre::Result<()> {
        assert_eq!(
            parse_gamekeys(ORDER_LIST)?,
            vec!["aAbBcC123", "dDeEfF456", "gGhHiI789"]
        );
        Ok(())
    }

    #[test]
    fn html_instead_of_order_list_means_signed_out() {
        let err = parse_gamekeys("<!DOCTYPE html><html>login</html>").err();
        assert!(err.is_some_and(|e| e.to_string().contains("Sign in again")));
    }

    #[test]
    fn reads_gamekeys_from_library_page() {
        assert_eq!(
            parse_library_page_gamekeys(LIBRARY_PAGE),
            Some(vec!["aAbBcC123".to_string(), "dDeEfF456".to_string()])
        );
        assert_eq!(parse_library_page_gamekeys("<html>Log in</html>"), None);
    }

    #[test]
    fn lists_drm_free_games_and_skips_keys_and_non_games() -> eyre::Result<()> {
        let games = owned_games(&parse_orders(ORDERS)?);
        // Sorted by title: Another Game, FEZ, World of Goo.
        assert_eq!(
            ids(&games),
            vec!["anothergame_store", "fez_indiebundle", "worldofgoo_linux"]
        );

        let fez = games.get(1);
        assert_eq!(fez.map(|g| g.title.as_str()), Some("FEZ"));
        assert_eq!(fez.and_then(|g| g.extra("gamekey")), Some("aAbBcC123"));
        assert_eq!(
            fez.and_then(|g| g.extra("url")),
            Some("https://www.humblebundle.com/downloads?key=aAbBcC123")
        );
        assert_eq!(
            fez.and_then(|g| g.extra("platforms")),
            Some("linux,mac,windows")
        );

        // A Linux-only game counts too (Playnite only takes Windows).
        let goo = games.get(2);
        assert_eq!(goo.and_then(|g| g.extra("platforms")), Some("linux"));
        Ok(())
    }

    #[test]
    fn same_game_in_two_orders_is_listed_once() -> eyre::Result<()> {
        let games = owned_games(&parse_orders(ORDERS)?);
        assert_eq!(games.iter().filter(|g| g.title == "FEZ").count(), 1);
        Ok(())
    }

    fn order(gamekey: &str, games: &[(&str, &str)], keys: &[&str]) -> Order {
        Order {
            gamekey: Some(gamekey.into()),
            subproducts: games
                .iter()
                .map(|(machine_name, human_name)| SubProduct {
                    machine_name: Some((*machine_name).into()),
                    human_name: Some((*human_name).into()),
                    downloads: vec![Download {
                        platform: Some("windows".into()),
                    }],
                })
                .collect(),
            tpkd_dict: Some(TpkdDict {
                all_tpks: keys
                    .iter()
                    .map(|name| Tpk {
                        human_name: Some((*name).into()),
                    })
                    .collect(),
            }),
        }
    }

    #[test]
    fn keys_only_hide_their_own_game_in_their_own_order() {
        let orders = [
            order("b", &[("limbo", "Limbo"), ("fez", "FEZ")], &[]),
            order(
                "a",
                &[("other", "Other")],
                &["Limbo Steam Key", "FEZ II Steam Key"],
            ),
            order(
                "c",
                &[("hue", "Hue"), ("braid", "Braid")],
                &["Huey Steam Key", "Braid (Steam)"],
            ),
        ];
        let games = owned_games(&orders);
        assert_eq!(ids(&games), vec!["fez", "hue", "limbo", "other"]);
    }

    #[test]
    fn a_new_order_does_not_change_a_games_id() {
        let first = owned_games(&[order("m", &[("game_b", "Game")], &[])]);
        let both = owned_games(&[
            order("a", &[("game_c", "Game")], &[]),
            order("m", &[("game_b", "Game")], &[]),
        ]);
        assert_eq!(ids(&first), ids(&both));
    }

    #[test]
    fn matches_key_names_like_playnite() {
        assert!(is_key_for("FEZ", "FEZ"));
        assert!(is_key_for("FEZ Steam Key", "FEZ"));
        assert!(is_key_for("Fez (Steam)", "FEZ"));
        assert!(is_key_for("FEZ™ - Epic Games Key", "FEZ"));
        assert!(!is_key_for("FEZ II Steam Key", "FEZ"));
        assert!(!is_key_for("Limbonauts Steam Key", "Limbo"));
        assert!(!is_key_for("Chuchel Steam Key", "Hue"));
        assert!(!is_key_for("Anything", ""));
    }

    #[test]
    fn reads_pasted_cookie() {
        assert_eq!(
            parse_pasted_cookie("  \"abc|123|def\"\n"),
            Some("\"abc|123|def\"".into())
        );
        assert_eq!(
            parse_pasted_cookie("_simpleauth_sess=\"abc|123\"; Path=/"),
            Some("\"abc|123\"".into())
        );
        assert_eq!(parse_pasted_cookie(""), None);
        assert_eq!(parse_pasted_cookie("\"\""), None);
        assert_eq!(parse_pasted_cookie("two words"), None);
    }

    #[test]
    fn prefers_webview_cookie() -> eyre::Result<()> {
        let capture = LoginCapture {
            final_url: Some("https://www.humblebundle.com/home/library".into()),
            cookies: vec![
                Cookie {
                    name: "csrf_cookie".into(),
                    value: "x".into(),
                    domain: "www.humblebundle.com".into(),
                },
                Cookie {
                    name: SESSION_COOKIE.into(),
                    value: "\"sess\"".into(),
                    domain: ".humblebundle.com".into(),
                },
            ],
            pasted: None,
        };
        assert_eq!(session_cookie_from_capture(&capture)?, "\"sess\"");
        assert!(session_cookie_from_capture(&LoginCapture::default()).is_err());
        Ok(())
    }

    #[test]
    fn picks_up_renewed_cookie() -> eyre::Result<()> {
        let mut headers = HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            "csrf_cookie=abc; Path=/".parse()?,
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            "_simpleauth_sess=\"new|1\"; Domain=.humblebundle.com; Path=/; HttpOnly".parse()?,
        );
        assert_eq!(renewed_session_cookie(&headers), Some("\"new|1\"".into()));

        let mut deleted = HeaderMap::new();
        deleted.append(
            reqwest::header::SET_COOKIE,
            "_simpleauth_sess=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT".parse()?,
        );
        assert_eq!(renewed_session_cookie(&deleted), None);
        Ok(())
    }

    #[test]
    fn strips_trademarks() {
        assert_eq!(clean_title(" Game™ Deluxe® "), "Game Deluxe");
    }
}
