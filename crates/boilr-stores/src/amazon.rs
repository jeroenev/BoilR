//! Amazon Games (Prime Gaming).
//!
//! Signs in the way the Amazon Games launcher does, as Playnite's AmazonGamesLibrary
//! (`Services/AmazonAccountClient.cs`) and nile (`nile/api/authorization.py`) do: an
//! OpenID sign-in page on amazon.com with a PKCE S256 challenge and a per-device client id,
//! then `api.amazon.com/auth/register` turns the authorization code into a device
//! registration with bearer tokens. The library comes from the launcher's entitlements
//! service (`gaming.amazon.com/api/distribution/entitlements`, nile `nile/api/library.py`).
//!
//! [`OwnedGame::id`] is the entitlement's `product.id` (`amzn1.adg.product.<uuid>`): the
//! `Id` column of the Amazon Games launcher's `GameInstallInfo.sqlite` (read by BoilR's
//! Amazon platform) and the `id` in nile's `installed.json` (read by BoilR's Heroic platform).
//!
//! `extra` keys:
//! - `entitlement_id`: the entitlement's own id (nile's `GetGameDownload` and the
//!   `AMAZON_GAMES_FUEL_ENTITLEMENT_ID` launch variable use it).
//! - `sku`: the product SKU, when the service sends one.
//! - `icon_url`: square product art, when the service sends one.

use reqwest::{header, Client};
use serde::{Deserialize, Serialize};

use crate::{http, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId};

const SIGN_IN_URL: &str = "https://www.amazon.com/ap/signin";
const RETURN_TO: &str = "https://www.amazon.com";
const REGISTER_URL: &str = "https://api.amazon.com/auth/register";
const TOKEN_URL: &str = "https://api.amazon.com/auth/token";
const ENTITLEMENTS_URL: &str = "https://gaming.amazon.com/api/distribution/entitlements";
const ENTITLEMENTS_TARGET: &str =
    "com.amazon.animusdistributionservice.entitlement.AnimusEntitlementsService.GetEntitlements";
/// Key id the launcher sends with GetEntitlements (nile `api/library.py`, Playnite
/// `AmazonAccountClient.cs`; taken from `Amazon.Fuel.Plugin.Entitlement.dll`).
const ENTITLEMENTS_KEY_ID: &str = "d5dc8b8b-86c8-4fc4-ae93-18c0def5314d";
/// Amazon's device type for the Amazon Games launcher on Windows.
const DEVICE_TYPE: &str = "A2UMVHOX7UP4V7";
const MARKETPLACE_ID: &str = "ATVPDKIKX0DER";
const ASSOC_HANDLE: &str = "amzn_sonic_games_launcher";
const APP_NAME: &str = "AGSLauncher for Windows";
const APP_VERSION: &str = "1.0.0";
const AUTH_USER_AGENT: &str = "AGSLauncher/1.0.0";
const LAUNCHER_USER_AGENT: &str = "com.amazon.agslauncher.win/3.0.9495.3";
const CODE_PARAM: &str = "openid.oa2.authorization_code";
/// Entitlements of this product line are in-game loot, not games (Playnite skips them).
const SKIPPED_PRODUCT_LINE: &str = "Twitch:FuelEntitlement";
/// Refresh this many seconds before the access token runs out.
const EXPIRY_MARGIN_SECS: u64 = 120;
/// Stops a misbehaving `nextToken` from looping forever.
const MAX_PAGES: usize = 200;

/// State carried from [`begin_login`] to [`complete_login`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LoginState {
    code_verifier: String,
    device_serial: String,
    client_id: String,
}

/// What [`Session::data`] holds for Amazon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmazonSession {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix time in seconds; 0 when Amazon did not say.
    #[serde(default)]
    pub expires_at: u64,
    /// The serial this device was registered with; also the base of `hardwareHash`.
    pub device_serial: String,
    pub client_id: String,
    #[serde(default)]
    pub user_id: Option<String>,
}

impl AmazonSession {
    fn needs_refresh(&self, now: u64) -> bool {
        self.access_token.is_empty() || self.expires_at <= now.saturating_add(EXPIRY_MARGIN_SECS)
    }
}

pub fn begin_login() -> PendingLogin {
    let state = new_login_state();
    let start_url = sign_in_url(&state.client_id, &code_challenge(&state.code_verifier));
    PendingLogin {
        store: StoreId::Amazon,
        start_url: start_url.clone(),
        done_url_prefixes: done_url_prefixes(),
        cookie_urls: vec![],
        manual: ManualLogin::Paste {
            url: start_url,
            instructions: "Sign in to Amazon in your browser. When it lands on the Amazon home \
                           page, copy the full address from the address bar and paste it here."
                .to_string(),
            placeholder:
                "https://www.amazon.com/?openid.assoc_handle=...&openid.oa2.authorization_code=..."
                    .to_string(),
        },
        user_agent: Some("Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) @amzn/aga-electron-platform/1.0.0 Chrome/78.0.3904.130 Electron/7.1.9 Safari/537.36".to_string()),
        state: serde_json::to_value(&state).unwrap_or_default(),
    }
}

pub async fn complete_login(
    client: &Client,
    pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Session> {
    let state: LoginState = serde_json::from_value(pending.state.clone()).map_err(|err| {
        eyre::format_err!("The Amazon sign-in lost its state ({err}). Start the sign-in again.")
    })?;
    let code = authorization_code(capture)?;
    let body = register_request(&code, &state);
    let response: RegisterResponse = http::send_json(
        client
            .post(REGISTER_URL)
            .header(header::USER_AGENT, AUTH_USER_AGENT)
            .json(&body),
    )
    .await?;
    let (session, account_name) = session_from_registration(response, &state, crate::now_unix())?;
    Ok(Session {
        account_name,
        data: serde_json::to_value(&session)?,
    })
}

pub async fn fetch_owned(client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut amazon: AmazonSession =
        serde_json::from_value(session.data.clone()).map_err(|err| {
            eyre::format_err!("The saved Amazon login can't be read ({err}). Sign in again.")
        })?;
    if amazon.needs_refresh(crate::now_unix()) {
        refresh(client, &mut amazon).await?;
        session.data = serde_json::to_value(&amazon)?;
    }

    let hardware_hash = hardware_hash(&amazon.device_serial);
    let mut entitlements = vec![];
    let mut next_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let body = entitlements_request(&hardware_hash, next_token.as_deref());
        let response: EntitlementsResponse = http::send_json(
            client
                .post(ENTITLEMENTS_URL)
                .header(header::USER_AGENT, LAUNCHER_USER_AGENT)
                .header("X-Amz-Target", ENTITLEMENTS_TARGET)
                .header("x-amzn-token", &amazon.access_token)
                .header(header::CONTENT_ENCODING, "amz-1.0")
                .json(&body),
        )
        .await?;
        let page = entitlements_page(response)?;
        entitlements.extend(page.entitlements);
        match page.next_token {
            Some(token) if next_token.as_deref() != Some(token.as_str()) => {
                next_token = Some(token)
            }
            _ => return Ok(owned_games(entitlements)),
        }
    }
    Err(eyre::format_err!(
        "Amazon kept sending more pages of games after {MAX_PAGES} pages; giving up."
    ))
}

async fn refresh(client: &Client, session: &mut AmazonSession) -> eyre::Result<()> {
    if session.refresh_token.is_empty() {
        return Err(eyre::format_err!(
            "The saved Amazon login has no refresh token. Sign in again."
        ));
    }
    let body = RefreshRequest {
        source_token: &session.refresh_token,
        source_token_type: "refresh_token",
        requested_token_type: "access_token",
        app_name: APP_NAME,
        app_version: APP_VERSION,
    };
    let response: RefreshResponse = http::send_json(
        client
            .post(TOKEN_URL)
            .header(header::USER_AGENT, AUTH_USER_AGENT)
            .json(&body),
    )
    .await
    .map_err(|err| err.wrap_err("Could not refresh the Amazon login"))?;
    apply_refresh(session, response, crate::now_unix())
}

// ---------------------------------------------------------------------------------------
// Login pieces (pure)
// ---------------------------------------------------------------------------------------

fn new_login_state() -> LoginState {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut verifier_bytes = [0u8; 32];
    rng.fill_bytes(&mut verifier_bytes);
    let mut serial_bytes = [0u8; 16];
    rng.fill_bytes(&mut serial_bytes);
    let device_serial = hex::encode_upper(serial_bytes);
    LoginState {
        code_verifier: code_verifier_from_bytes(&verifier_bytes),
        client_id: client_id(&device_serial),
        device_serial,
    }
}

/// PKCE verifier: 32 random bytes, base64url without padding (43 characters), like nile.
fn code_verifier_from_bytes(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 challenge: base64url (no padding) of the SHA-256 of the verifier.
fn code_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier))
}

/// The OAuth client id of a device: hex of `<serial>#<device type>` (nile `generate_client_id`).
fn client_id(device_serial: &str) -> String {
    hex::encode(format!("{device_serial}#{DEVICE_TYPE}"))
}

/// `hardwareHash` for GetEntitlements: upper-case hex SHA-256 of the device serial (nile).
fn hardware_hash(device_serial: &str) -> String {
    use sha2::Digest;
    hex::encode_upper(sha2::Sha256::digest(device_serial))
}

fn sign_in_url(client_id: &str, challenge: &str) -> String {
    let device_client_id = format!("device:{client_id}");
    let params = [
        ("openid.ns", "http://specs.openid.net/auth/2.0"),
        (
            "openid.claimed_id",
            "http://specs.openid.net/auth/2.0/identifier_select",
        ),
        (
            "openid.identity",
            "http://specs.openid.net/auth/2.0/identifier_select",
        ),
        ("openid.mode", "checkid_setup"),
        ("openid.oa2.scope", "device_auth_access"),
        ("openid.ns.oa2", "http://www.amazon.com/ap/ext/oauth/2"),
        ("openid.oa2.response_type", "code"),
        ("openid.oa2.code_challenge_method", "S256"),
        ("openid.oa2.client_id", device_client_id.as_str()),
        ("language", "en_US"),
        ("marketPlaceId", MARKETPLACE_ID),
        ("openid.return_to", RETURN_TO),
        ("openid.pape.max_auth_age", "0"),
        ("openid.assoc_handle", ASSOC_HANDLE),
        ("pageId", ASSOC_HANDLE),
        ("openid.oa2.code_challenge", challenge),
    ];
    match url::Url::parse_with_params(SIGN_IN_URL, params) {
        Ok(url) => url.to_string(),
        // SIGN_IN_URL is a valid constant, so this does not happen.
        Err(_) => SIGN_IN_URL.to_string(),
    }
}

/// Amazon sends the browser back to `openid.return_to` with the OpenID response appended:
/// `https://www.amazon.com/?openid.assoc_handle=amzn_sonic_games_launcher&...`. The sign-in
/// pages themselves live under `/ap/`, so they never match.
fn done_url_prefixes() -> Vec<String> {
    vec![
        "https://www.amazon.com/?openid.".to_string(),
        "https://amazon.com/?openid.".to_string(),
    ]
}

/// The authorization code from the redirect URL (webview) or from what the user pasted:
/// the full URL, or the bare code.
fn authorization_code(capture: &LoginCapture) -> eyre::Result<String> {
    let raw = capture
        .final_url
        .as_deref()
        .or(capture.pasted.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| eyre::format_err!("The Amazon sign-in did not return anything."))?;

    if raw.contains("://") {
        if let Some(code) = http::query_param(raw, CODE_PARAM).filter(|c| !c.is_empty()) {
            return Ok(code);
        }
        if http::query_param(raw, "openid.mode").as_deref() == Some("cancel") {
            return Err(eyre::format_err!("The Amazon sign-in was cancelled."));
        }
        return Err(eyre::format_err!(
            "That address has no Amazon sign-in code ({CODE_PARAM}). Sign in again and copy \
             the full address the browser lands on after signing in."
        ));
    }
    if raw.contains(char::is_whitespace) || raw.contains('?') || raw.contains('&') {
        return Err(eyre::format_err!(
            "That does not look like the address Amazon sent you to. Paste the full address \
             from the browser's address bar."
        ));
    }
    Ok(raw.to_string())
}

#[derive(Serialize)]
struct RegisterRequest<'a> {
    auth_data: RegisterAuthData<'a>,
    registration_data: RegistrationData<'a>,
    requested_extensions: [&'a str; 2],
    requested_token_type: [&'a str; 2],
    user_context_map: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct RegisterAuthData<'a> {
    authorization_code: &'a str,
    client_domain: &'a str,
    client_id: &'a str,
    code_algorithm: &'a str,
    code_verifier: &'a str,
    use_global_authentication: bool,
}

#[derive(Serialize)]
struct RegistrationData<'a> {
    app_name: &'a str,
    app_version: &'a str,
    device_model: &'a str,
    device_name: Option<&'a str>,
    device_serial: &'a str,
    device_type: &'a str,
    domain: &'a str,
    os_version: &'a str,
}

fn register_request<'a>(code: &'a str, state: &'a LoginState) -> RegisterRequest<'a> {
    RegisterRequest {
        auth_data: RegisterAuthData {
            authorization_code: code,
            client_domain: "DeviceLegacy",
            client_id: &state.client_id,
            code_algorithm: "SHA-256",
            code_verifier: &state.code_verifier,
            use_global_authentication: false,
        },
        registration_data: RegistrationData {
            app_name: APP_NAME,
            app_version: APP_VERSION,
            device_model: "Windows",
            device_name: None,
            device_serial: &state.device_serial,
            device_type: DEVICE_TYPE,
            domain: "Device",
            os_version: "10.0.19044.0",
        },
        requested_extensions: ["customer_info", "device_info"],
        requested_token_type: ["bearer", "mac_dms"],
        user_context_map: serde_json::Map::new(),
    }
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    response: Option<RegisterResponseBody>,
}

#[derive(Debug, Deserialize)]
struct RegisterResponseBody {
    success: Option<RegisterSuccess>,
    error: Option<AmazonError>,
}

#[derive(Debug, Deserialize)]
struct AmazonError {
    code: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterSuccess {
    tokens: RegisterTokens,
    #[serde(default)]
    extensions: Option<RegisterExtensions>,
    #[serde(default)]
    customer_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterTokens {
    bearer: BearerToken,
}

#[derive(Debug, Deserialize)]
struct BearerToken {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: Option<Seconds>,
}

#[derive(Debug, Deserialize)]
struct RegisterExtensions {
    #[serde(default)]
    customer_info: Option<CustomerInfo>,
    #[serde(default)]
    device_info: Option<DeviceInfo>,
}

#[derive(Debug, Deserialize)]
struct CustomerInfo {
    user_id: Option<String>,
    name: Option<String>,
    given_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceInfo {
    device_serial_number: Option<String>,
}

/// Amazon sends `expires_in` as a number in some responses and as a string in others.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(u64),
    Text(String),
}

impl Seconds {
    fn value(&self) -> Option<u64> {
        match self {
            Seconds::Number(n) => Some(*n),
            Seconds::Text(s) => s.trim().parse().ok(),
        }
    }
}

fn expires_at(expires_in: Option<&Seconds>, now: u64) -> u64 {
    expires_in
        .and_then(Seconds::value)
        .map(|secs| now.saturating_add(secs))
        .unwrap_or_default()
}

/// Turns the device registration into the stored session and the account name.
fn session_from_registration(
    response: RegisterResponse,
    state: &LoginState,
    now: u64,
) -> eyre::Result<(AmazonSession, Option<String>)> {
    let body = response
        .response
        .ok_or_else(|| eyre::format_err!("Amazon's device registration returned no response."))?;
    let success = match (body.success, body.error) {
        (Some(success), _) => success,
        (None, Some(error)) => {
            return Err(eyre::format_err!(
                "Amazon refused to register BoilR: {} ({}). Sign in again.",
                error.message.unwrap_or_else(|| "no reason given".into()),
                error.code.unwrap_or_else(|| "no code".into())
            ))
        }
        (None, None) => {
            return Err(eyre::format_err!(
                "Amazon's device registration returned neither tokens nor an error."
            ))
        }
    };
    let bearer = success.tokens.bearer;
    if bearer.access_token.is_empty() || bearer.refresh_token.is_empty() {
        return Err(eyre::format_err!(
            "Amazon's device registration returned empty tokens."
        ));
    }
    let extensions = success.extensions;
    let (customer, device) = match extensions {
        Some(ext) => (ext.customer_info, ext.device_info),
        None => (None, None),
    };
    let device_serial = device
        .and_then(|d| d.device_serial_number)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.device_serial.clone());
    let (user_id, account_name) = match customer {
        Some(c) => (
            c.user_id,
            c.name
                .filter(|n| !n.trim().is_empty())
                .or(c.given_name.filter(|n| !n.trim().is_empty())),
        ),
        None => (None, None),
    };
    let session = AmazonSession {
        expires_at: expires_at(bearer.expires_in.as_ref(), now),
        access_token: bearer.access_token,
        refresh_token: bearer.refresh_token,
        device_serial,
        client_id: state.client_id.clone(),
        user_id: user_id.or(success.customer_id),
    };
    Ok((session, account_name))
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    source_token: &'a str,
    source_token_type: &'a str,
    requested_token_type: &'a str,
    app_name: &'a str,
    app_version: &'a str,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<Seconds>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn apply_refresh(
    session: &mut AmazonSession,
    response: RefreshResponse,
    now: u64,
) -> eyre::Result<()> {
    match response.access_token.filter(|t| !t.is_empty()) {
        Some(token) => {
            session.access_token = token;
            session.expires_at = expires_at(response.expires_in.as_ref(), now);
            Ok(())
        }
        None => Err(eyre::format_err!(
            "Amazon did not refresh the login ({}). Sign in again.",
            response
                .error_description
                .or(response.error)
                .unwrap_or_else(|| "no access token in the response".into())
        )),
    }
}

// ---------------------------------------------------------------------------------------
// Library pieces (pure)
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntitlementsRequest<'a> {
    #[serde(rename = "Operation")]
    operation: &'a str,
    client_id: &'a str,
    sync_point: Option<u64>,
    next_token: Option<&'a str>,
    max_results: u32,
    product_id_filter: Option<&'a str>,
    key_id: &'a str,
    hardware_hash: &'a str,
}

fn entitlements_request<'a>(
    hardware_hash: &'a str,
    next_token: Option<&'a str>,
) -> EntitlementsRequest<'a> {
    EntitlementsRequest {
        operation: "GetEntitlements",
        client_id: "Sonic",
        sync_point: None,
        next_token,
        max_results: 50,
        product_id_filter: None,
        key_id: ENTITLEMENTS_KEY_ID,
        hardware_hash,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntitlementsResponse {
    entitlements: Option<Vec<Entitlement>>,
    next_token: Option<String>,
    #[serde(rename = "__type")]
    error_type: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entitlement {
    id: Option<String>,
    product: Option<Product>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    id: Option<String>,
    title: Option<String>,
    product_line: Option<String>,
    sku: Option<String>,
    product_detail: Option<ProductDetail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductDetail {
    icon_url: Option<String>,
}

struct EntitlementsPage {
    entitlements: Vec<Entitlement>,
    next_token: Option<String>,
}

fn entitlements_page(response: EntitlementsResponse) -> eyre::Result<EntitlementsPage> {
    match response.entitlements {
        Some(entitlements) => Ok(EntitlementsPage {
            entitlements,
            next_token: response.next_token.filter(|t| !t.is_empty()),
        }),
        None => Err(eyre::format_err!(
            "Amazon did not send the list of games: {}{}",
            response
                .error_type
                .map(|t| format!("{t}: "))
                .unwrap_or_default(),
            response.message.unwrap_or_else(|| "no reason given".into())
        )),
    }
}

/// Games from entitlements: skips in-game loot and entries without a product id or title,
/// and keeps the first entitlement of each product.
fn owned_games(entitlements: Vec<Entitlement>) -> Vec<OwnedGame> {
    let mut seen = std::collections::HashSet::new();
    let mut games = vec![];
    for entitlement in entitlements {
        let Some(product) = entitlement.product else {
            continue;
        };
        if product.product_line.as_deref() == Some(SKIPPED_PRODUCT_LINE) {
            continue;
        }
        let Some(id) = product.id.filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        let Some(title) = product
            .title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let mut game = OwnedGame::new(StoreId::Amazon, id, title);
        if let Some(entitlement_id) = entitlement.id.filter(|s| !s.is_empty()) {
            game = game.with_extra("entitlement_id", entitlement_id);
        }
        if let Some(sku) = product.sku.filter(|s| !s.is_empty()) {
            game = game.with_extra("sku", sku);
        }
        if let Some(icon) = product
            .product_detail
            .and_then(|d| d.icon_url)
            .filter(|s| !s.is_empty())
        {
            game = game.with_extra("icon_url", icon);
        }
        games.push(game);
    }
    games
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> LoginState {
        LoginState {
            code_verifier: "verifier".into(),
            device_serial: "0123456789ABCDEF0123456789ABCDEF".into(),
            client_id: client_id("0123456789ABCDEF0123456789ABCDEF"),
        }
    }

    fn capture_url(url: &str) -> LoginCapture {
        LoginCapture {
            final_url: Some(url.into()),
            ..Default::default()
        }
    }

    #[test]
    fn pkce_matches_rfc_7636_example() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn login_state_has_the_expected_shape() {
        let state = new_login_state();
        assert_eq!(state.code_verifier.len(), 43);
        assert!(state
            .code_verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(state.device_serial.len(), 32);
        assert!(state
            .device_serial
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()));
        assert_eq!(state.client_id, client_id(&state.device_serial));
        let challenge = code_challenge(&state.code_verifier);
        assert_eq!(challenge.len(), 43);
        assert!(!challenge.contains('='));
        assert_ne!(new_login_state(), state);
    }

    #[test]
    fn client_id_is_hex_of_serial_and_device_type() {
        // Playnite's fixed client id decodes to this serial.
        assert_eq!(
            client_id("73db8d286c2a924d2e77e1af67766645"),
            "3733646238643238366332613932346432653737653161663637373636363435234132554d56484f58375550345637"
        );
    }

    #[test]
    fn hardware_hash_is_upper_hex_sha256() {
        assert_eq!(
            hardware_hash("abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn sign_in_url_carries_client_and_challenge() {
        let pending = begin_login();
        let state: LoginState =
            serde_json::from_value(pending.state.clone()).unwrap_or_else(|_| LoginState {
                code_verifier: String::new(),
                device_serial: String::new(),
                client_id: String::new(),
            });
        assert!(!state.code_verifier.is_empty());
        let url = &pending.start_url;
        assert!(url.starts_with("https://www.amazon.com/ap/signin?"));
        assert_eq!(
            http::query_param(url, "openid.oa2.client_id"),
            Some(format!("device:{}", state.client_id))
        );
        assert_eq!(
            http::query_param(url, "openid.oa2.code_challenge"),
            Some(code_challenge(&state.code_verifier))
        );
        assert_eq!(
            http::query_param(url, "openid.oa2.code_challenge_method").as_deref(),
            Some("S256")
        );
        assert_eq!(
            http::query_param(url, "openid.assoc_handle").as_deref(),
            Some("amzn_sonic_games_launcher")
        );
        assert!(matches!(pending.manual, ManualLogin::Paste { .. }));
    }

    #[test]
    fn done_prefixes_match_only_the_final_redirect() {
        let pending = begin_login();
        let matches = |url: &str| {
            pending
                .done_url_prefixes
                .iter()
                .any(|p| url.starts_with(p.as_str()))
        };
        assert!(!matches(&pending.start_url));
        assert!(!matches("https://www.amazon.com/ap/signin?openid.ns=x"));
        assert!(!matches("https://www.amazon.com/ap/mfa?arb=x"));
        assert!(!matches("https://www.amazon.com/ap/cvf/request?arb=x"));
        assert!(!matches("https://www.amazon.com/"));
        assert!(matches(
            "https://www.amazon.com/?openid.assoc_handle=amzn_sonic_games_launcher&openid.oa2.authorization_code=ANabc"
        ));
    }

    #[test]
    fn reads_the_code_from_the_redirect() -> eyre::Result<()> {
        let url = "https://www.amazon.com/?openid.assoc_handle=amzn_sonic_games_launcher\
                   &openid.mode=id_res&openid.oa2.authorization_code=ANcodeXYZ&openid.ns=x";
        assert_eq!(authorization_code(&capture_url(url))?, "ANcodeXYZ");

        let pasted = LoginCapture {
            pasted: Some(format!("  {url}\n")),
            ..Default::default()
        };
        assert_eq!(authorization_code(&pasted)?, "ANcodeXYZ");

        let bare = LoginCapture {
            pasted: Some("ANcodeXYZ".into()),
            ..Default::default()
        };
        assert_eq!(authorization_code(&bare)?, "ANcodeXYZ");
        Ok(())
    }

    #[test]
    fn rejects_redirects_without_a_code() {
        let cancelled = capture_url("https://www.amazon.com/?openid.mode=cancel");
        let err = authorization_code(&cancelled).map_err(|e| e.to_string());
        assert_eq!(err, Err("The Amazon sign-in was cancelled.".to_string()));
        assert!(
            authorization_code(&capture_url("https://www.amazon.com/?openid.mode=id_res")).is_err()
        );
        assert!(authorization_code(&LoginCapture::default()).is_err());
        let junk = LoginCapture {
            pasted: Some("not a code at all".into()),
            ..Default::default()
        };
        assert!(authorization_code(&junk).is_err());
    }

    #[test]
    fn register_request_has_launcher_shape() -> eyre::Result<()> {
        let state = state();
        let json = serde_json::to_value(register_request("ANcode", &state))?;
        let at = |pointer: &str| json.pointer(pointer).cloned().unwrap_or_default();
        assert_eq!(at("/auth_data/authorization_code"), "ANcode");
        assert_eq!(at("/auth_data/client_id"), state.client_id.as_str());
        assert_eq!(at("/auth_data/code_verifier"), "verifier");
        assert_eq!(at("/auth_data/code_algorithm"), "SHA-256");
        assert_eq!(at("/auth_data/client_domain"), "DeviceLegacy");
        assert_eq!(at("/registration_data/device_type"), "A2UMVHOX7UP4V7");
        assert_eq!(
            at("/registration_data/device_serial"),
            state.device_serial.as_str()
        );
        assert_eq!(
            at("/requested_token_type"),
            serde_json::json!(["bearer", "mac_dms"])
        );
        assert_eq!(at("/user_context_map"), serde_json::json!({}));
        Ok(())
    }

    #[test]
    fn parses_a_device_registration() -> eyre::Result<()> {
        let response: RegisterResponse =
            serde_json::from_str(include_str!("../testdata/amazon/register_success.json"))?;
        let (session, name) = session_from_registration(response, &state(), 1000)?;
        assert_eq!(name.as_deref(), Some("Jane Player"));
        assert_eq!(session.access_token, "Atna|access");
        assert_eq!(session.refresh_token, "Atnr|refresh");
        assert_eq!(session.expires_at, 4600);
        assert_eq!(session.device_serial, "SERIALFROMAMAZON");
        assert_eq!(session.client_id, state().client_id);
        assert_eq!(session.user_id.as_deref(), Some("amzn1.account.TEST"));
        assert!(!session.needs_refresh(1000));
        assert!(session.needs_refresh(4500));
        Ok(())
    }

    #[test]
    fn reports_a_refused_registration() -> eyre::Result<()> {
        let response: RegisterResponse =
            serde_json::from_str(include_str!("../testdata/amazon/register_error.json"))?;
        let err = session_from_registration(response, &state(), 0)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("Authorization code is invalid"), "{err}");
        assert!(err.contains("InvalidValue"), "{err}");
        Ok(())
    }

    #[test]
    fn applies_a_token_refresh() -> eyre::Result<()> {
        let response: RegisterResponse =
            serde_json::from_str(include_str!("../testdata/amazon/register_success.json"))?;
        let (mut session, _) = session_from_registration(response, &state(), 0)?;
        let refresh: RefreshResponse =
            serde_json::from_str(include_str!("../testdata/amazon/refresh.json"))?;
        apply_refresh(&mut session, refresh, 10_000)?;
        assert_eq!(session.access_token, "Atna|fresh");
        assert_eq!(session.refresh_token, "Atnr|refresh");
        assert_eq!(session.expires_at, 13_600);

        let failed: RefreshResponse = serde_json::from_str(
            r#"{"error":"invalid_grant","error_description":"The request has an invalid grant parameter"}"#,
        )?;
        let err = apply_refresh(&mut session, failed, 0)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("invalid grant"), "{err}");
        assert_eq!(session.access_token, "Atna|fresh");
        Ok(())
    }

    #[test]
    fn entitlements_request_has_launcher_shape() -> eyre::Result<()> {
        let json = serde_json::to_value(entitlements_request("HASH", Some("next")))?;
        assert_eq!(
            json,
            serde_json::json!({
                "Operation": "GetEntitlements",
                "clientId": "Sonic",
                "syncPoint": null,
                "nextToken": "next",
                "maxResults": 50,
                "productIdFilter": null,
                "keyId": "d5dc8b8b-86c8-4fc4-ae93-18c0def5314d",
                "hardwareHash": "HASH"
            })
        );
        Ok(())
    }

    #[test]
    fn parses_entitlement_pages_into_games() -> eyre::Result<()> {
        let first: EntitlementsResponse =
            serde_json::from_str(include_str!("../testdata/amazon/entitlements_page1.json"))?;
        let second: EntitlementsResponse =
            serde_json::from_str(include_str!("../testdata/amazon/entitlements_page2.json"))?;
        let first = entitlements_page(first)?;
        let second = entitlements_page(second)?;
        assert_eq!(first.next_token.as_deref(), Some("page-2-token"));
        assert_eq!(second.next_token, None);

        let mut all = first.entitlements;
        all.extend(second.entitlements);
        let games = owned_games(all);
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "amzn1.adg.product.11111111-1111-1111-1111-111111111111",
                "amzn1.adg.product.33333333-3333-3333-3333-333333333333",
            ]
        );
        let [fallout, other] = games.as_slice() else {
            eyre::bail!("expected two games, got {}", games.len());
        };
        assert_eq!(fallout.store, StoreId::Amazon);
        assert_eq!(fallout.title, "Fallout: New Vegas Ultimate Edition");
        assert_eq!(
            fallout.extra("entitlement_id"),
            Some("aaaaaaaa-0000-0000-0000-000000000001")
        );
        assert_eq!(fallout.extra("sku"), Some("FNV_SKU"));
        assert_eq!(
            fallout.extra("icon_url"),
            Some("https://example.invalid/fnv-icon.jpg")
        );
        assert_eq!(other.title, "Second Game");
        assert_eq!(other.extra("sku"), None);
        Ok(())
    }

    #[test]
    fn reports_an_entitlements_error() -> eyre::Result<()> {
        let response: EntitlementsResponse = serde_json::from_str(
            r#"{"__type":"com.amazon.coral.service#UnrecognizedClientException","message":"Invalid token"}"#,
        )?;
        let err = entitlements_page(response)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err.contains("UnrecognizedClientException: Invalid token"),
            "{err}"
        );
        Ok(())
    }

    #[test]
    fn session_round_trips_through_json() -> eyre::Result<()> {
        let session = AmazonSession {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 5,
            device_serial: "S".into(),
            client_id: "C".into(),
            user_id: None,
        };
        let value = serde_json::to_value(&session)?;
        assert_eq!(serde_json::from_value::<AmazonSession>(value)?, session);
        Ok(())
    }
}
