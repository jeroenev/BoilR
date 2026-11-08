//! Epic Games Store: signs in with the Epic Games Launcher's OAuth client and lists the
//! account's library through Epic's library and catalog services, the way Playnite's
//! EpicLibrary plugin and legendary do.
//!
//! `OwnedGame.id` is Epic's `appName`, the id the launcher's install manifests, legendary and
//! Heroic use (`src/platforms/egs/manifest_item.rs`, `src/platforms/heroic/`).
//!
//! `extra` keys:
//! - `namespace`: the catalog namespace.
//! - `catalog_item_id`: the catalog item id.
//! - `third_party`: the store that manages the game, when Epic only hands out a key for
//!   another launcher (`ThirdPartyManagedApp`, for example "Origin", "The EA App"; falls back
//!   to `ThirdPartyManagedProvider`, for example "UbisoftConnect"). Absent for Epic games.
//!
//! `namespace`, `catalog_item_id` and `id` together form the launcher URI
//! `com.epicgames.launcher://apps/{namespace}%3A{catalog_item_id}%3A{id}?action=launch&silent=true`.

use std::collections::{BTreeMap, BTreeSet};

use reqwest::{
    header::{ACCEPT, AUTHORIZATION, COOKIE, USER_AGENT},
    Client, RequestBuilder, Response, StatusCode,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::{
    http, now_unix, Cookie, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId,
};

/// The Epic Games Launcher's OAuth client ("launcherAppClient2"), as used by Playnite
/// (`EpicAccountClient.cs`) and legendary (`api/egs.py`).
const CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";
const CLIENT_SECRET: &str = "daafbccc737745039dffe53d94fc76cf";
/// legendary's launcher user agent for the account, library and catalog services.
const LAUNCHER_USER_AGENT: &str =
    "UELauncher/11.0.1-14907503+++Portal+Release-Live Windows/10.0.19041.1.256.64bit";

const LOGIN_URL: &str = "https://www.epicgames.com/id/login";
const REDIRECT_URL: &str = "https://www.epicgames.com/id/api/redirect";
const OAUTH_TOKEN_URL: &str =
    "https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token";
const ACCOUNT_URL: &str =
    "https://account-public-service-prod03.ol.epicgames.com/account/api/public/account/";
const LIBRARY_ITEMS_URL: &str =
    "https://library-service.live.use1a.on.epicgames.com/library/api/public/items";
const CATALOG_URL: &str =
    "https://catalog-public-service-prod06.ol.epicgames.com/catalog/api/shared/namespace/";

/// Catalog item ids asked for in one bulk request (ids are 32 characters; this keeps URLs short).
const CATALOG_BATCH_SIZE: usize = 50;
/// Catalog requests in flight at once.
const CATALOG_CONCURRENCY: usize = 8;
/// A guard against a library service that keeps handing out cursors.
const MAX_LIBRARY_PAGES: usize = 1000;
/// Refresh the access token when it expires within this many seconds.
const REFRESH_MARGIN_SECS: u64 = 300;

/// The page that hands out an authorization code for the launcher client once the user is
/// signed in at epicgames.com. It shows JSON with `authorizationCode`.
fn redirect_url() -> String {
    format!("{REDIRECT_URL}?clientId={CLIENT_ID}&responseType=code")
}

/// Epic's login page, sending the user on to [`redirect_url`] afterwards (legendary's
/// `get_auth_url`). A user who is already signed in goes straight to the code.
fn login_url() -> String {
    let redirect: String =
        url::form_urlencoded::byte_serialize(redirect_url().as_bytes()).collect();
    format!("{LOGIN_URL}?redirectUrl={redirect}")
}

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::Epic,
        start_url: login_url(),
        done_url_prefixes: vec![
            REDIRECT_URL.to_string(),
            "https://epicgames.com/id/api/redirect".to_string(),
            // Where Epic's login page sends the real launcher (Playnite, Heroic). The code is
            // in the `code` query parameter.
            "http://localhost/launcher/authorized".to_string(),
            "https://localhost/launcher/authorized".to_string(),
        ],
        cookie_urls: vec!["https://www.epicgames.com".to_string()],
        manual: ManualLogin::Paste {
            url: login_url(),
            instructions: "Sign in to Epic Games in your browser. Epic then shows a page of text \
                           containing \"authorizationCode\". Copy the code after it (or all of \
                           the text) and paste it here. The code works for a few minutes only."
                .to_string(),
            placeholder: "authorizationCode".to_string(),
        },
        user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) EpicGamesLauncher".to_string()),
        state: serde_json::Value::Null,
    }
}

pub async fn complete_login(
    client: &Client,
    _pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Session> {
    let pasted = capture.pasted.as_deref().filter(|p| !p.trim().is_empty());
    let code = match (pasted, capture.final_url.as_deref()) {
        (Some(pasted), _) => parse_pasted_code(pasted)?,
        (None, Some(final_url)) => code_from_webview(client, final_url, &capture.cookies).await?,
        (None, None) => {
            return Err(eyre::format_err!(
                "The Epic login did not finish. Sign in again."
            ))
        }
    };

    let tokens = request_token(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("token_type", "eg1"),
        ],
    )
    .await?;
    let mut data = EpicSession::from_tokens(tokens, None, now_unix());
    if data.display_name.is_none() {
        // Only the name shown in the UI; a failure here should not fail the login.
        data.display_name = fetch_display_name(client, &data).await.ok().flatten();
    }
    Ok(Session {
        account_name: data.display_name.clone(),
        data: serde_json::to_value(&data)?,
    })
}

pub async fn fetch_owned(client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut data: EpicSession = serde_json::from_value(session.data.clone())
        .map_err(|_| eyre::format_err!("The saved Epic login is unreadable. Sign in again."))?;
    if data.needs_refresh(now_unix()) {
        refresh(client, &mut data, session).await?;
    }

    // The first page doubles as the token check: a token Epic revoked early gets one refresh.
    let first = library_request(client, &data.access_token, None)
        .send()
        .await?;
    let first: LibraryPage = if first.status() == StatusCode::UNAUTHORIZED {
        refresh(client, &mut data, session).await?;
        http::send_json(library_request(client, &data.access_token, None)).await?
    } else {
        read_json(first).await?
    };

    let mut records = first.records;
    let mut cursor = first.response_metadata.and_then(|m| m.next_cursor);
    let mut seen_cursors = BTreeSet::new();
    while let Some(next) = cursor.filter(|c| !c.is_empty()) {
        if !seen_cursors.insert(next.clone()) || seen_cursors.len() > MAX_LIBRARY_PAGES {
            break;
        }
        let page: LibraryPage =
            http::send_json(library_request(client, &data.access_token, Some(&next))).await?;
        records.extend(page.records);
        cursor = page.response_metadata.and_then(|m| m.next_cursor);
    }

    let entries = library_entries(records);
    let catalog = fetch_catalog(client, &data.access_token, catalog_batches(&entries)).await?;
    Ok(owned_games(&entries, &catalog))
}

// ---------------------------------------------------------------------------------------------
// Login

/// The JSON the redirect page shows.
#[derive(Debug, Deserialize)]
struct RedirectResponse {
    #[serde(rename = "authorizationCode", default)]
    authorization_code: Option<String>,
}

/// Reads the authorization code from the redirect page's JSON.
fn parse_redirect_response(body: &str) -> eyre::Result<String> {
    let response: RedirectResponse = serde_json::from_str(body.trim()).map_err(|_| {
        eyre::format_err!(
            "Epic did not return a login code. Body starts with: {}",
            http::snippet(body)
        )
    })?;
    response
        .authorization_code
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            eyre::format_err!(
                "Epic gave no login code because this browser is not signed in to epicgames.com. \
                 Sign in, open the link again and paste what it shows."
            )
        })
}

/// Accepts what a user may paste: the bare code, the redirect page's JSON (whole or just the
/// `"authorizationCode":"…"` part), or a `…/launcher/authorized?code=…` URL.
fn parse_pasted_code(pasted: &str) -> eyre::Result<String> {
    let text = pasted.trim();
    if text.starts_with('{') {
        return parse_redirect_response(text);
    }
    if text.contains("authorizationCode") {
        return parse_redirect_response(&format!("{{{}}}", text.trim_end_matches(',')));
    }
    if let Some(code) = http::query_param(text, "code").filter(|c| looks_like_code(c)) {
        return Ok(code);
    }
    let code = text.trim_matches(|c| c == '"' || c == '\'').trim();
    if looks_like_code(code) {
        return Ok(code.to_string());
    }
    Err(eyre::format_err!(
        "That does not look like an Epic login code. Paste the value of \"authorizationCode\" \
         (32 letters and digits) or all of the text the Epic page shows."
    ))
}

fn looks_like_code(text: &str) -> bool {
    (16..=128).contains(&text.len()) && text.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The login window stopped at the redirect page (or at the launcher's localhost callback).
/// The code is in the URL for the callback; otherwise the redirect page is fetched with the
/// window's cookies.
async fn code_from_webview(
    client: &Client,
    final_url: &str,
    cookies: &[Cookie],
) -> eyre::Result<String> {
    if let Some(code) = http::query_param(final_url, "code").filter(|c| looks_like_code(c)) {
        return Ok(code);
    }
    let cookie = http::cookie_header(cookies, "www.epicgames.com");
    if cookie.is_empty() {
        return Err(eyre::format_err!(
            "The login window returned no Epic cookies. Sign in again."
        ));
    }
    // Always our own URL, so the cookies only ever go to epicgames.com with our client id.
    let response = http::send(
        client
            .get(redirect_url())
            .header(COOKIE, cookie)
            .header(ACCEPT, "application/json"),
    )
    .await?;
    parse_redirect_response(&response.text().await?)
}

/// Tokens as Epic's OAuth service returns them.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: String,
    #[serde(default)]
    refresh_expires: u64,
    #[serde(default)]
    account_id: String,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
}

/// Epic's error body, shared by all its services.
#[derive(Debug, Default, Deserialize)]
struct EpicError {
    #[serde(rename = "errorCode", default)]
    error_code: Option<String>,
    #[serde(rename = "errorMessage", default)]
    error_message: Option<String>,
    #[serde(rename = "continuationUrl", default)]
    continuation_url: Option<String>,
}

fn parse_token_response(success: bool, body: &str) -> eyre::Result<TokenResponse> {
    if let Ok(error) = serde_json::from_str::<EpicError>(body) {
        if let Some(code) = error.error_code.as_deref().filter(|c| !c.is_empty()) {
            return Err(token_error(code, &error));
        }
    }
    if !success {
        return Err(eyre::format_err!(
            "Epic sign-in failed: {}",
            http::snippet(body)
        ));
    }
    serde_json::from_str(body).map_err(|err| {
        eyre::format_err!(
            "Unexpected answer from Epic's sign-in service: {err}. Body starts with: {}",
            http::snippet(body)
        )
    })
}

fn token_error(code: &str, error: &EpicError) -> eyre::Report {
    let message = error.error_message.as_deref().unwrap_or(code);
    if code.ends_with("corrective_action_required") {
        let url = error
            .continuation_url
            .as_deref()
            .unwrap_or("https://www.epicgames.com/account");
        return eyre::format_err!(
            "Epic needs you to confirm something on your account first ({message}). Open {url} \
             in a browser, then sign in again."
        );
    }
    if code.contains("authorization_code") {
        return eyre::format_err!(
            "Epic did not accept the login code: it expired or was already used. Sign in again."
        );
    }
    if code.contains("refresh_token") || code.contains("invalid_grant") {
        return eyre::format_err!("Your Epic sign-in has expired. Sign in again.");
    }
    eyre::format_err!("Epic sign-in failed: {message} ({code})")
}

async fn request_token(client: &Client, form: &[(&str, &str)]) -> eyre::Result<TokenResponse> {
    let response = client
        .post(OAUTH_TOKEN_URL)
        .basic_auth(CLIENT_ID, Some(CLIENT_SECRET))
        .header(USER_AGENT, LAUNCHER_USER_AGENT)
        .form(form)
        .send()
        .await
        .map_err(|err| eyre::format_err!("Could not reach Epic's sign-in service: {err}"))?;
    let success = response.status().is_success();
    let body = response.text().await?;
    parse_token_response(success, &body)
}

/// What BoilR keeps in `Session.data` for Epic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct EpicSession {
    access_token: String,
    /// Unix time in seconds.
    access_expires_at: u64,
    refresh_token: String,
    /// Unix time in seconds.
    refresh_expires_at: u64,
    account_id: String,
    #[serde(default)]
    display_name: Option<String>,
}

impl EpicSession {
    fn from_tokens(tokens: TokenResponse, previous_name: Option<String>, now: u64) -> Self {
        Self {
            access_token: tokens.access_token,
            access_expires_at: now.saturating_add(tokens.expires_in),
            refresh_token: tokens.refresh_token,
            refresh_expires_at: now.saturating_add(tokens.refresh_expires),
            account_id: tokens.account_id,
            display_name: tokens
                .display_name
                .filter(|n| !n.trim().is_empty())
                .or(previous_name),
        }
    }

    fn needs_refresh(&self, now: u64) -> bool {
        now.saturating_add(REFRESH_MARGIN_SECS) >= self.access_expires_at
    }
}

/// Trades the refresh token for new tokens and writes them into the session.
async fn refresh(
    client: &Client,
    data: &mut EpicSession,
    session: &mut Session,
) -> eyre::Result<()> {
    let tokens = request_token(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &data.refresh_token),
            ("token_type", "eg1"),
        ],
    )
    .await?;
    let mut refreshed = EpicSession::from_tokens(tokens, data.display_name.clone(), now_unix());
    if refreshed.account_id.is_empty() {
        refreshed.account_id = data.account_id.clone();
    }
    *data = refreshed;
    session.data = serde_json::to_value(&*data)?;
    if data.display_name.is_some() {
        session.account_name = data.display_name.clone();
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
}

async fn fetch_display_name(client: &Client, data: &EpicSession) -> eyre::Result<Option<String>> {
    if data.account_id.is_empty() {
        return Ok(None);
    }
    let url = format!("{ACCOUNT_URL}{}", data.account_id);
    let account: AccountResponse =
        http::send_json(authorized(client.get(url), &data.access_token)).await?;
    Ok(account.display_name.filter(|n| !n.trim().is_empty()))
}

// ---------------------------------------------------------------------------------------------
// Library

fn authorized(request: RequestBuilder, access_token: &str) -> RequestBuilder {
    request
        .header(AUTHORIZATION, format!("bearer {access_token}"))
        .header(USER_AGENT, LAUNCHER_USER_AGENT)
}

fn library_request(client: &Client, access_token: &str, cursor: Option<&str>) -> RequestBuilder {
    // Playnite asks for the Windows platform; legendary sends no platform.
    let mut query = vec![("includeMetadata", "true"), ("platform", "Windows")];
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor));
    }
    authorized(client.get(LIBRARY_ITEMS_URL), access_token).query(&query)
}

/// Like [`http::send_json`] for a response already received.
async fn read_json<T: DeserializeOwned>(response: Response) -> eyre::Result<T> {
    let status = response.status();
    let url = response.url().to_string();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(eyre::format_err!(
            "{status} from {url}: {}",
            http::snippet(&text)
        ));
    }
    serde_json::from_str(&text).map_err(|err| {
        eyre::format_err!(
            "Unexpected response from {url}: {err}. Body starts with: {}",
            http::snippet(&text)
        )
    })
}

#[derive(Debug, Default, Deserialize)]
struct LibraryPage {
    #[serde(rename = "responseMetadata", default)]
    response_metadata: Option<ResponseMetadata>,
    #[serde(default)]
    records: Vec<LibraryRecord>,
}

#[derive(Debug, Default, Deserialize)]
struct ResponseMetadata {
    #[serde(rename = "nextCursor", default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LibraryRecord {
    #[serde(rename = "appName", default)]
    app_name: Option<String>,
    #[serde(rename = "catalogItemId", default)]
    catalog_item_id: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(rename = "sandboxType", default)]
    sandbox_type: Option<String>,
}

/// A library record worth looking up in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LibraryEntry {
    app_name: String,
    namespace: String,
    catalog_item_id: String,
}

/// Drops what Playnite and legendary skip before the catalog lookup: Unreal Engine
/// marketplace assets (namespace `ue`), private sandboxes (unreleased builds), records without
/// an app name, and legendary's broken app name "1". Keeps the first record per app name.
fn library_entries(records: Vec<LibraryRecord>) -> Vec<LibraryEntry> {
    let mut seen = BTreeSet::new();
    records
        .into_iter()
        .filter_map(|record| {
            let app_name = record.app_name.filter(|a| !a.is_empty() && a != "1")?;
            let namespace = record
                .namespace
                .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("ue"))?;
            let catalog_item_id = record.catalog_item_id.filter(|c| !c.is_empty())?;
            if record
                .sandbox_type
                .is_some_and(|s| s.eq_ignore_ascii_case("PRIVATE"))
            {
                return None;
            }
            Some(LibraryEntry {
                app_name,
                namespace,
                catalog_item_id,
            })
        })
        .filter(|entry| seen.insert(entry.app_name.clone()))
        .collect()
}

/// Catalog lookups grouped per namespace (the bulk endpoint is per namespace), at most
/// [`CATALOG_BATCH_SIZE`] ids each.
fn catalog_batches(entries: &[LibraryEntry]) -> Vec<(String, Vec<String>)> {
    let mut per_namespace: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for entry in entries {
        per_namespace
            .entry(&entry.namespace)
            .or_default()
            .insert(&entry.catalog_item_id);
    }
    per_namespace
        .into_iter()
        .flat_map(|(namespace, ids)| {
            let ids: Vec<String> = ids.into_iter().map(str::to_string).collect();
            ids.chunks(CATALOG_BATCH_SIZE)
                .map(|chunk| (namespace.to_string(), chunk.to_vec()))
                .collect::<Vec<_>>()
        })
        .collect()
}

type CatalogKey = (String, String);

/// Looks up catalog items, [`CATALOG_CONCURRENCY`] requests at a time. Ids a batched request
/// did not answer are asked for one by one, so this works whether or not the service honours
/// several `id` parameters in one request.
async fn fetch_catalog(
    client: &Client,
    access_token: &str,
    batches: Vec<(String, Vec<String>)>,
) -> eyre::Result<BTreeMap<CatalogKey, CatalogItem>> {
    let mut found = run_catalog_requests(client, access_token, batches.clone()).await?;
    let mut retries = Vec::new();
    for (namespace, ids) in batches.into_iter().filter(|(_, ids)| ids.len() > 1) {
        for id in ids {
            if !found.contains_key(&(namespace.clone(), id.clone())) {
                retries.push((namespace.clone(), vec![id]));
            }
        }
    }
    if !retries.is_empty() {
        found.extend(run_catalog_requests(client, access_token, retries).await?);
    }
    Ok(found)
}

async fn run_catalog_requests(
    client: &Client,
    access_token: &str,
    batches: Vec<(String, Vec<String>)>,
) -> eyre::Result<BTreeMap<CatalogKey, CatalogItem>> {
    let mut found = BTreeMap::new();
    let mut pending = batches.into_iter();
    let mut tasks = JoinSet::new();
    loop {
        while tasks.len() < CATALOG_CONCURRENCY {
            let Some((namespace, ids)) = pending.next() else {
                break;
            };
            let client = client.clone();
            let access_token = access_token.to_string();
            tasks.spawn(async move {
                let items = fetch_catalog_batch(&client, &access_token, &namespace, &ids).await?;
                Ok::<_, eyre::Report>((namespace, items))
            });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        // Returning drops `tasks`, which cancels the requests still running.
        let (namespace, items) =
            joined.map_err(|err| eyre::format_err!("Looking up Epic games failed: {err}"))??;
        for (id, item) in items {
            found.insert((namespace.clone(), id), item);
        }
    }
    Ok(found)
}

async fn fetch_catalog_batch(
    client: &Client,
    access_token: &str,
    namespace: &str,
    ids: &[String],
) -> eyre::Result<BTreeMap<String, CatalogItem>> {
    let mut url = url::Url::parse(CATALOG_URL)?;
    url.path_segments_mut()
        .map_err(|_| eyre::format_err!("Bad Epic catalog URL"))?
        .pop_if_empty()
        .push(namespace)
        .push("bulk")
        .push("items");
    let mut query: Vec<(&str, &str)> = ids.iter().map(|id| ("id", id.as_str())).collect();
    query.extend([
        ("includeDLCDetails", "false"),
        ("includeMainGameDetails", "true"),
        ("country", "US"),
        ("locale", "en-US"),
    ]);
    let raw: BTreeMap<String, serde_json::Value> =
        http::send_json(authorized(client.get(url), access_token).query(&query)).await?;
    Ok(parse_catalog(raw))
}

#[derive(Clone, Debug, Default, Deserialize)]
struct CatalogItem {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    categories: Option<Vec<Category>>,
    #[serde(rename = "mainGameItem", default)]
    main_game_item: Option<serde_json::Value>,
    #[serde(rename = "customAttributes", default)]
    custom_attributes: Option<BTreeMap<String, CustomAttribute>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Category {
    #[serde(default)]
    path: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct CustomAttribute {
    #[serde(default)]
    value: Option<String>,
}

impl CatalogItem {
    fn has_category(&self, path: &str) -> bool {
        self.categories
            .iter()
            .flatten()
            .any(|c| c.path.eq_ignore_ascii_case(path))
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.custom_attributes
            .as_ref()?
            .get(name)?
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    /// legendary's `third_party_store`.
    fn third_party(&self) -> Option<&str> {
        self.attribute("ThirdPartyManagedApp")
            .or_else(|| self.attribute("ThirdPartyManagedProvider"))
    }
}

/// Keeps the items that have the catalog's shape; one odd item must not hide the others.
fn parse_catalog(raw: BTreeMap<String, serde_json::Value>) -> BTreeMap<String, CatalogItem> {
    raw.into_iter()
        .filter_map(|(id, value)| Some((id, serde_json::from_value(value).ok()?)))
        .collect()
}

/// Turns a library entry into a game, skipping what Playnite's `GetLibraryGames` skips: items
/// that are not applications, DLC that cannot be launched on its own, digital extras
/// (soundtracks, art books) and engine plugins. legendary also skips mods.
fn owned_game(entry: &LibraryEntry, item: &CatalogItem) -> Option<OwnedGame> {
    if !item.has_category("applications") {
        return None;
    }
    if item.main_game_item.as_ref().is_some_and(|m| !m.is_null())
        && !item.has_category("addons/launchable")
    {
        return None;
    }
    if ["digitalextras", "plugins", "plugins/engine", "mods"]
        .iter()
        .any(|c| item.has_category(c))
    {
        return None;
    }
    let title = clean_title(item.title.as_deref()?);
    if title.is_empty() {
        return None;
    }
    let game = OwnedGame::new(StoreId::Epic, entry.app_name.clone(), title)
        .with_extra("namespace", entry.namespace.clone())
        .with_extra("catalog_item_id", entry.catalog_item_id.clone());
    Some(match item.third_party() {
        Some(store) => game.with_extra("third_party", store),
        None => game,
    })
}

fn owned_games(
    entries: &[LibraryEntry],
    catalog: &BTreeMap<CatalogKey, CatalogItem>,
) -> Vec<OwnedGame> {
    entries
        .iter()
        .filter_map(|entry| {
            let key = (entry.namespace.clone(), entry.catalog_item_id.clone());
            owned_game(entry, catalog.get(&key)?)
        })
        .collect()
}

/// Playnite's `RemoveTrademarks`.
fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '™' | '®' | '©'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = include_str!("../testdata/epic/token_response.json");
    const TOKEN_CODE_USED: &str = include_str!("../testdata/epic/token_error_code_used.json");
    const TOKEN_CORRECTIVE: &str = include_str!("../testdata/epic/token_error_corrective.json");
    const REDIRECT_IN: &str = include_str!("../testdata/epic/redirect_signed_in.json");
    const REDIRECT_OUT: &str = include_str!("../testdata/epic/redirect_signed_out.json");
    const LIBRARY_1: &str = include_str!("../testdata/epic/library_page1.json");
    const LIBRARY_2: &str = include_str!("../testdata/epic/library_page2.json");
    const CATALOG: &str = include_str!("../testdata/epic/catalog_bulk.json");

    #[test]
    fn login_urls_never_match_done_prefixes() -> eyre::Result<()> {
        let pending = begin_login();
        let manual_url = match &pending.manual {
            ManualLogin::Paste { url, .. } => url.clone(),
            ManualLogin::Unsupported => String::new(),
        };
        assert!(!manual_url.is_empty());
        for url in [&pending.start_url, &manual_url] {
            assert!(pending
                .done_url_prefixes
                .iter()
                .all(|p| !url.starts_with(p)));
        }
        assert_eq!(
            http::query_param(&pending.start_url, "redirectUrl"),
            Some(redirect_url())
        );
        assert!(redirect_url().starts_with(REDIRECT_URL));
        assert!(redirect_url().contains(CLIENT_ID));
        Ok(())
    }

    #[test]
    fn reads_code_from_redirect_page() -> eyre::Result<()> {
        assert_eq!(
            parse_redirect_response(REDIRECT_IN)?,
            "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
        );
        let err = parse_redirect_response(REDIRECT_OUT)
            .err()
            .map(|e| e.to_string());
        assert!(err.is_some_and(|e| e.contains("not signed in")));
        assert!(parse_redirect_response("<html>").is_err());
        Ok(())
    }

    #[test]
    fn accepts_every_pasted_form() -> eyre::Result<()> {
        let code = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
        assert_eq!(parse_pasted_code(code)?, code);
        assert_eq!(parse_pasted_code(&format!("  \"{code}\"\n"))?, code);
        assert_eq!(parse_pasted_code(REDIRECT_IN)?, code);
        assert_eq!(
            parse_pasted_code(&format!("\"authorizationCode\":\"{code}\","))?,
            code
        );
        assert_eq!(
            parse_pasted_code(&format!(
                "https://localhost/launcher/authorized?code={code}"
            ))?,
            code
        );
        assert!(parse_pasted_code("not a code!").is_err());
        assert!(parse_pasted_code("abc").is_err());
        assert!(parse_pasted_code(REDIRECT_OUT).is_err());
        Ok(())
    }

    #[test]
    fn parses_tokens_into_a_session() -> eyre::Result<()> {
        let tokens = parse_token_response(true, TOKEN)?;
        let session = EpicSession::from_tokens(tokens, None, 1_000);
        assert_eq!(session.access_token, "eg1~access");
        assert_eq!(session.refresh_token, "eg1~refresh");
        assert_eq!(session.access_expires_at, 8_200);
        assert_eq!(session.refresh_expires_at, 1_988_200);
        assert_eq!(session.account_id, "0123456789abcdef0123456789abcdef");
        assert_eq!(session.display_name.as_deref(), Some("Player One"));

        assert!(!session.needs_refresh(1_000));
        assert!(session.needs_refresh(8_200 - REFRESH_MARGIN_SECS));
        assert!(session.needs_refresh(9_000));

        let stored: EpicSession = serde_json::from_value(serde_json::to_value(&session)?)?;
        assert_eq!(stored, session);
        Ok(())
    }

    #[test]
    fn refresh_keeps_the_known_name() -> eyre::Result<()> {
        let mut tokens = parse_token_response(true, TOKEN)?;
        tokens.display_name = None;
        let session = EpicSession::from_tokens(tokens, Some("Old Name".into()), 0);
        assert_eq!(session.display_name.as_deref(), Some("Old Name"));
        Ok(())
    }

    #[test]
    fn explains_token_errors() {
        let used = parse_token_response(false, TOKEN_CODE_USED)
            .err()
            .map(|e| e.to_string());
        assert!(used.is_some_and(|e| e.contains("expired or was already used")));

        let corrective = parse_token_response(false, TOKEN_CORRECTIVE)
            .err()
            .map(|e| e.to_string());
        assert!(corrective.is_some_and(|e| e.contains("continuation=abc")));

        let refresh = parse_token_response(
            false,
            r#"{"errorCode":"errors.com.epicgames.account.auth_token.invalid_refresh_token","errorMessage":"Sorry the refresh token is invalid"}"#,
        )
        .err()
        .map(|e| e.to_string());
        assert!(refresh.is_some_and(|e| e.contains("Sign in again")));

        let outage = parse_token_response(false, "<html>Service Unavailable</html>")
            .err()
            .map(|e| e.to_string());
        assert!(outage.is_some_and(|e| e.contains("Service Unavailable")));
    }

    fn library() -> eyre::Result<Vec<LibraryEntry>> {
        let first: LibraryPage = serde_json::from_str(LIBRARY_1)?;
        let second: LibraryPage = serde_json::from_str(LIBRARY_2)?;
        assert_eq!(
            first
                .response_metadata
                .and_then(|m| m.next_cursor)
                .as_deref(),
            Some("cursor-page-2")
        );
        assert!(second
            .response_metadata
            .and_then(|m| m.next_cursor)
            .is_none());
        let mut records = first.records;
        records.extend(second.records);
        Ok(library_entries(records))
    }

    #[test]
    fn filters_library_records() -> eyre::Result<()> {
        let names: Vec<String> = library()?.into_iter().map(|e| e.app_name).collect();
        assert_eq!(
            names,
            [
                "Fortnite",
                "Calluna",
                "CallunaDLC1",
                "CallunaExpansionLauncher",
                "EAGame",
                "UbiGame",
                "Soundtrack",
                "EnginePlugin",
                "NotAnApplication",
            ]
        );
        Ok(())
    }

    #[test]
    fn batches_catalog_lookups_per_namespace() -> eyre::Result<()> {
        let batches = catalog_batches(&library()?);
        let control = batches.iter().find(|(ns, _)| ns == "ns-control");
        assert_eq!(
            control.map(|(_, ids)| ids.clone()),
            Some(vec![
                "cat-control".to_string(),
                "cat-control-dlc".to_string(),
                "cat-control-launchable".to_string(),
            ])
        );
        assert_eq!(batches.len(), 7);

        let many: Vec<LibraryEntry> = (0..120)
            .map(|i| LibraryEntry {
                app_name: format!("app{i}"),
                namespace: "ns".into(),
                catalog_item_id: format!("id{i}"),
            })
            .collect();
        let sizes: Vec<usize> = catalog_batches(&many)
            .iter()
            .map(|(_, ids)| ids.len())
            .collect();
        assert_eq!(sizes, [50, 50, 20]);
        Ok(())
    }

    #[test]
    fn turns_catalog_items_into_games() -> eyre::Result<()> {
        let raw: BTreeMap<String, serde_json::Value> = serde_json::from_str(CATALOG)?;
        let items = parse_catalog(raw);
        assert!(items.contains_key("cat-fortnite"));
        assert!(!items.contains_key("cat-broken"));

        let entries = library()?;
        let catalog: BTreeMap<CatalogKey, CatalogItem> = entries
            .iter()
            .filter_map(|e| {
                let item = items.get(&e.catalog_item_id)?.clone();
                Some(((e.namespace.clone(), e.catalog_item_id.clone()), item))
            })
            .collect();
        let games = owned_games(&entries, &catalog);

        let summary: Vec<(&str, &str, Option<&str>)> = games
            .iter()
            .map(|g| (g.id.as_str(), g.title.as_str(), g.extra("third_party")))
            .collect();
        assert_eq!(
            summary,
            [
                ("Fortnite", "Fortnite", None),
                ("Calluna", "Control Ultimate Edition", None),
                (
                    "CallunaExpansionLauncher",
                    "Control Standalone Expansion",
                    None
                ),
                ("EAGame", "Star Wars Jedi: Fallen Order", Some("The EA App")),
                (
                    "UbiGame",
                    "Assassin's Creed Valhalla",
                    Some("UbisoftConnect")
                ),
            ]
        );

        let control = games.iter().find(|g| g.id == "Calluna");
        assert_eq!(
            control.and_then(|g| g.extra("namespace")),
            Some("ns-control")
        );
        assert_eq!(
            control.and_then(|g| g.extra("catalog_item_id")),
            Some("cat-control")
        );
        assert!(games.iter().all(|g| g.store == StoreId::Epic));
        Ok(())
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(
            clean_title("  Control™  Ultimate ®Edition "),
            "Control Ultimate Edition"
        );
    }
}
