//! GOG: sign in with the OAuth flow of the GOG Galaxy client and list the owned games.
//!
//! Login: the window opens `auth.gog.com/auth` for the Galaxy client id; after sign-in GOG
//! redirects to `embed.gog.com/on_login_success?origin=client&code=...`. The code is
//! exchanged at `auth.gog.com/token` for an access token (valid about an hour), a refresh
//! token and the user id. Same client id, secret and redirect as Heroic's gogdl
//! (`gogdl/auth.py`) and Minigalaxy (`minigalaxy/api.py`).
//!
//! Library: `embed.gog.com/account/getFilteredProducts?mediaType=1&page=N` with the Bearer
//! token (what Minigalaxy uses; Playnite reads the same endpoint on `www.gog.com` with web
//! cookies). `mediaType=1` leaves out movies; DLC is not listed on its own, except DLC whose
//! base game the account does not own (`isBaseProductMissing`), which is skipped.
//!
//! [`OwnedGame::extra`] keys:
//! - `slug`: the store slug (`https://www.gog.com/game/<slug>`), when GOG sends one.

use reqwest::Client;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{http, now_unix, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId};

/// The GOG Galaxy client's OAuth client. Public: shipped in Galaxy and used by Heroic's
/// gogdl and Minigalaxy.
const CLIENT_ID: &str = "46899977096215655";
const CLIENT_SECRET: &str = "9d85c43b1482497dbbce61f6e4aa173a433796eeae2ca8c5f6129f2dc4de46d9";
const REDIRECT_URI: &str = "https://embed.gog.com/on_login_success?origin=client";
/// Where GOG sends the browser after sign-in. The login pages themselves live on
/// `auth.gog.com` and `login.gog.com`, so they never match.
const DONE_PREFIX: &str = "https://embed.gog.com/on_login_success";
const AUTH_URL: &str = "https://auth.gog.com/auth";
const TOKEN_URL: &str = "https://auth.gog.com/token";
const PRODUCTS_URL: &str = "https://embed.gog.com/account/getFilteredProducts";

/// Refresh the access token when it expires within this many seconds.
const REFRESH_MARGIN_SECS: u64 = 120;
/// Guards against a server that keeps reporting more pages.
const MAX_PAGES: u32 = 500;

/// Products GOG files as games that are only goodies (comics, artbooks, wallpapers).
/// List from Minigalaxy (`minigalaxy/constants.py`, `IGNORE_GAME_IDS`).
const NOT_GAMES: [&str; 6] = [
    "1424856371", // Hotline Miami 2: Wrong Number - Digital Comics
    "1980301910", // The Witcher Goodies Collection
    "2005648906", // Spring Sale Goodies Collection #1
    "1486144755", // Cyberpunk 2077 Goodies Collection
    "1581684020", // A Plague Tale Digital Goodies Pack
    "1185685769", // CDPR Goodie Pack Content
];

/// What BoilR keeps in [`Session::data`] for GOG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GogSession {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    /// Unix time in seconds when `access_token` stops working.
    pub expires_at: u64,
}

/// Response of `auth.gog.com/token` (for both the code and the refresh grant).
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(deserialize_with = "string_or_number")]
    user_id: String,
    expires_in: u64,
}

/// Response of `users.gog.com/users/<id>`; only the name is used.
#[derive(Debug, Deserialize)]
struct UserResponse {
    username: Option<String>,
}

/// One page of `account/getFilteredProducts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductsPage {
    #[serde(default)]
    page: u32,
    #[serde(default)]
    total_pages: u32,
    #[serde(default)]
    products: Vec<Product>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    #[serde(deserialize_with = "string_or_number")]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    is_game: Option<bool>,
    #[serde(default)]
    is_movie: Option<bool>,
    #[serde(default)]
    is_base_product_missing: Option<bool>,
}

pub fn begin_login() -> PendingLogin {
    let start_url = login_url();
    PendingLogin {
        store: StoreId::Gog,
        start_url: start_url.clone(),
        done_url_prefixes: vec![DONE_PREFIX.to_string()],
        cookie_urls: vec![],
        manual: ManualLogin::Paste {
            url: start_url,
            instructions: "Sign in to GOG in your browser. When the page turns blank and its \
                           address starts with embed.gog.com/on_login_success, copy the whole \
                           address from the address bar and paste it here."
                .to_string(),
            placeholder: "https://embed.gog.com/on_login_success?origin=client&code=..."
                .to_string(),
        },
        state: serde_json::Value::Null,
    }
}

fn login_url() -> String {
    let mut url = String::from(AUTH_URL);
    url.push('?');
    url.push_str(
        &url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", CLIENT_ID)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("response_type", "code")
            .append_pair("layout", "galaxy")
            .finish(),
    );
    url
}

pub async fn complete_login(
    client: &Client,
    _pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Session> {
    let code = code_from_capture(capture)?;
    let token: TokenResponse = http::send_json(client.get(TOKEN_URL).query(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", REDIRECT_URI),
    ]))
    .await
    .map_err(|err| eyre::format_err!("Could not finish the GOG sign-in: {err}"))?;
    let gog = session_from_token(token, now_unix());
    // The name is only for display; a failure here should not fail the login.
    let account_name = fetch_username(client, &gog).await.ok().flatten();
    Ok(Session {
        account_name,
        data: serde_json::to_value(&gog)?,
    })
}

pub async fn fetch_owned(client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut gog = read_session(session)?;
    if needs_refresh(&gog, now_unix()) {
        gog = refresh(client, &gog).await?;
        session.data = serde_json::to_value(&gog)?;
    }
    if session.account_name.is_none() {
        session.account_name = fetch_username(client, &gog).await.ok().flatten();
    }

    let mut games = Vec::new();
    let mut page = 1;
    loop {
        let body = http::send(
            client
                .get(PRODUCTS_URL)
                .bearer_auth(&gog.access_token)
                .query(&[("mediaType", "1"), ("page", &page.to_string())]),
        )
        .await
        .map_err(|err| eyre::format_err!("Could not load the GOG library: {err}"))?
        .text()
        .await?;
        let response = parse_products_page(&body)?;
        games.extend(games_from_page(&response));
        match next_page(&response, page) {
            Some(next) if next <= MAX_PAGES => page = next,
            _ => break,
        }
    }
    Ok(games)
}

fn read_session(session: &Session) -> eyre::Result<GogSession> {
    serde_json::from_value(session.data.clone()).map_err(|err| {
        eyre::format_err!("The saved GOG login is unreadable ({err}). Sign in again.")
    })
}

async fn refresh(client: &Client, gog: &GogSession) -> eyre::Result<GogSession> {
    let token: TokenResponse = http::send_json(client.get(TOKEN_URL).query(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "refresh_token"),
        ("refresh_token", gog.refresh_token.as_str()),
    ]))
    .await
    .map_err(|err| eyre::format_err!("The GOG login has expired; sign in again. ({err})"))?;
    Ok(session_from_token(token, now_unix()))
}

async fn fetch_username(client: &Client, gog: &GogSession) -> eyre::Result<Option<String>> {
    let user: UserResponse = http::send_json(
        client
            .get(format!("https://users.gog.com/users/{}", gog.user_id))
            .bearer_auth(&gog.access_token),
    )
    .await?;
    Ok(user.username.filter(|name| !name.trim().is_empty()))
}

fn session_from_token(token: TokenResponse, now: u64) -> GogSession {
    GogSession {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user_id: token.user_id,
        expires_at: now.saturating_add(token.expires_in),
    }
}

fn needs_refresh(gog: &GogSession, now: u64) -> bool {
    now.saturating_add(REFRESH_MARGIN_SECS) >= gog.expires_at
}

/// The authorization code from a webview redirect or from what the user pasted: the whole
/// redirect URL, its query string, or the bare code.
fn code_from_capture(capture: &LoginCapture) -> eyre::Result<String> {
    let input = capture
        .final_url
        .as_deref()
        .or(capture.pasted.as_deref())
        .map(str::trim)
        .unwrap_or_default();
    if input.is_empty() {
        return Err(eyre::format_err!(
            "Nothing to finish the GOG sign-in with. Paste the address of the page GOG shows after you sign in."
        ));
    }
    let query = match input.split_once('?') {
        Some((_, query)) => Some(query),
        None if input.contains('=') => Some(input),
        None => None,
    };
    let Some(query) = query else {
        if input
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == ':')
        {
            return Err(eyre::format_err!(
                "That does not look like the GOG sign-in address. Paste the whole address that starts with {DONE_PREFIX}."
            ));
        }
        return Ok(input.to_string());
    };
    let query = query.split('#').next().unwrap_or_default();
    let param = |name: &str| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    if let Some(code) = param("code") {
        return Ok(code);
    }
    if let Some(error) = param("error") {
        let detail = param("error_description").unwrap_or(error);
        return Err(eyre::format_err!(
            "GOG did not complete the sign-in: {detail}"
        ));
    }
    Err(eyre::format_err!(
        "The GOG sign-in address has no code in it. Paste the whole address that starts with {DONE_PREFIX}."
    ))
}

/// Without a valid token GOG redirects the library request to its sign-in web page.
fn parse_products_page(body: &str) -> eyre::Result<ProductsPage> {
    if body.trim_start().starts_with('<') {
        return Err(eyre::format_err!(
            "GOG did not accept the saved login (it sent a web page instead of the library). Sign in again."
        ));
    }
    serde_json::from_str(body).map_err(|err| {
        eyre::format_err!(
            "Unexpected GOG library response: {err}. Body starts with: {}",
            http::snippet(body)
        )
    })
}

fn games_from_page(page: &ProductsPage) -> Vec<OwnedGame> {
    page.products
        .iter()
        .filter(|p| is_owned_game(p))
        .map(|p| {
            let game = OwnedGame::new(StoreId::Gog, p.id.clone(), p.title.trim());
            match p.slug.as_deref().map(str::trim) {
                Some(slug) if !slug.is_empty() => game.with_extra("slug", slug),
                _ => game,
            }
        })
        .collect()
}

fn is_owned_game(product: &Product) -> bool {
    !product.id.is_empty()
        && !product.title.trim().is_empty()
        && product.is_movie != Some(true)
        && product.is_game != Some(false)
        && product.is_base_product_missing != Some(true)
        && !NOT_GAMES.contains(&product.id.as_str())
}

/// The page to fetch after `current`, if any.
fn next_page(page: &ProductsPage, current: u32) -> Option<u32> {
    let current = current.max(page.page);
    (!page.products.is_empty() && current < page.total_pages).then_some(current + 1)
}

/// GOG sends ids as numbers in some responses and as strings in others.
fn string_or_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Text(String),
        Number(u64),
    }
    Ok(match Id::deserialize(deserializer)? {
        Id::Text(text) => text,
        Id::Number(number) => number.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = include_str!("../testdata/gog/token.json");
    const PAGE_1: &str = include_str!("../testdata/gog/products_page1.json");
    const PAGE_2: &str = include_str!("../testdata/gog/products_page2.json");
    const USER: &str = include_str!("../testdata/gog/user.json");

    fn capture_url(url: &str) -> LoginCapture {
        LoginCapture {
            final_url: Some(url.to_string()),
            ..Default::default()
        }
    }

    fn capture_paste(text: &str) -> LoginCapture {
        LoginCapture {
            pasted: Some(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn login_window_starts_outside_the_done_prefix() {
        let pending = begin_login();
        assert!(pending.start_url.starts_with("https://auth.gog.com/auth?"));
        assert!(pending.start_url.contains("client_id=46899977096215655"));
        assert!(pending.start_url.contains(
            "redirect_uri=https%3A%2F%2Fembed.gog.com%2Fon_login_success%3Forigin%3Dclient"
        ));
        for prefix in &pending.done_url_prefixes {
            assert!(!pending.start_url.starts_with(prefix));
            assert!(!"https://login.gog.com/login".starts_with(prefix));
            assert!(REDIRECT_URI.starts_with(prefix));
        }
        assert!(matches!(pending.manual, ManualLogin::Paste { .. }));
    }

    #[test]
    fn reads_the_code_from_the_redirect() -> eyre::Result<()> {
        let url = "https://embed.gog.com/on_login_success?origin=client&code=AbC-123_x";
        assert_eq!(code_from_capture(&capture_url(url))?, "AbC-123_x");
        assert_eq!(
            code_from_capture(&capture_paste(&format!("  {url}\n")))?,
            "AbC-123_x"
        );
        assert_eq!(
            code_from_capture(&capture_paste("origin=client&code=XYZ"))?,
            "XYZ"
        );
        assert_eq!(code_from_capture(&capture_paste("XYZ123"))?, "XYZ123");
        assert_eq!(
            code_from_capture(&capture_paste(
                "embed.gog.com/on_login_success?origin=client&code=Q#frag"
            ))?,
            "Q"
        );
        Ok(())
    }

    #[test]
    fn rejects_input_without_a_code() {
        assert!(code_from_capture(&LoginCapture::default()).is_err());
        assert!(code_from_capture(&capture_paste("https://www.gog.com/en/")).is_err());
        assert!(code_from_capture(&capture_paste("not a code")).is_err());
        let denied = code_from_capture(&capture_url(
            "https://embed.gog.com/on_login_success?origin=client&error=access_denied&error_description=User+cancelled",
        ));
        assert!(denied.is_err_and(|e| e.to_string().contains("User cancelled")));
    }

    #[test]
    fn builds_a_session_from_the_token_response() -> eyre::Result<()> {
        let token: TokenResponse = serde_json::from_str(TOKEN)?;
        let gog = session_from_token(token, 1_000);
        assert_eq!(gog.access_token, "access-1");
        assert_eq!(gog.refresh_token, "refresh-1");
        assert_eq!(gog.user_id, "48628349957132247");
        assert_eq!(gog.expires_at, 4_600);
        assert!(!needs_refresh(&gog, 1_000));
        assert!(needs_refresh(&gog, 4_500));
        assert!(needs_refresh(&gog, 5_000));

        let session = Session {
            account_name: None,
            data: serde_json::to_value(&gog)?,
        };
        assert_eq!(read_session(&session)?, gog);
        Ok(())
    }

    #[test]
    fn numeric_user_id_is_accepted() -> eyre::Result<()> {
        let token: TokenResponse = serde_json::from_str(
            r#"{"access_token":"a","refresh_token":"r","user_id":42,"expires_in":3600}"#,
        )?;
        assert_eq!(token.user_id, "42");
        Ok(())
    }

    #[test]
    fn unreadable_session_asks_to_sign_in_again() {
        let session = Session {
            account_name: None,
            data: serde_json::json!({"cookies": []}),
        };
        assert!(read_session(&session).is_err_and(|e| e.to_string().contains("Sign in again")));
    }

    #[test]
    fn reads_the_username() -> eyre::Result<()> {
        let user: UserResponse = serde_json::from_str(USER)?;
        assert_eq!(user.username.as_deref(), Some("player_one"));
        Ok(())
    }

    #[test]
    fn keeps_games_and_skips_movies_dlc_and_goodies() -> eyre::Result<()> {
        let page: ProductsPage = serde_json::from_str(PAGE_1)?;
        let games = games_from_page(&page);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["1207658924", "1207664663", "1423049311"]);

        let unreal = games.iter().find(|g| g.id == "1207658924");
        assert_eq!(unreal.map(|g| g.title.as_str()), Some("Unreal Gold"));
        assert_eq!(unreal.and_then(|g| g.extra("slug")), Some("unreal_gold"));
        assert!(games.iter().all(|g| g.store == StoreId::Gog));
        // No slug in the response: no `slug` key.
        let no_slug = games.iter().find(|g| g.id == "1423049311");
        assert_eq!(no_slug.and_then(|g| g.extra("slug")), None);
        Ok(())
    }

    #[test]
    fn follows_pages_until_the_last() -> eyre::Result<()> {
        let first = parse_products_page(PAGE_1)?;
        let second = parse_products_page(PAGE_2)?;
        assert_eq!(next_page(&first, 1), Some(2));
        assert_eq!(next_page(&second, 2), None);
        assert_eq!(games_from_page(&second).len(), 1);

        let empty: ProductsPage =
            serde_json::from_str(r#"{"page":1,"totalPages":0,"products":[]}"#)?;
        assert_eq!(next_page(&empty, 1), None);
        assert!(games_from_page(&empty).is_empty());

        // A page that claims more pages but is empty stops the loop.
        let stuck: ProductsPage =
            serde_json::from_str(r#"{"page":3,"totalPages":9,"products":[]}"#)?;
        assert_eq!(next_page(&stuck, 3), None);
        Ok(())
    }

    #[test]
    fn sign_in_page_instead_of_library_asks_to_sign_in_again() {
        let html = "<!DOCTYPE html>\n<html><head><meta http-equiv=\"refresh\" content=\"0;url='/en##openlogin'\" /></head></html>";
        assert!(parse_products_page(html).is_err_and(|e| e.to_string().contains("Sign in again")));
        assert!(parse_products_page("{\"products\": 3}").is_err());
    }
}
