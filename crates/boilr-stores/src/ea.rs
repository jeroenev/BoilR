//! EA app. Follows Jeshibu's EaLibrary for Playnite
//! (github.com/Jeshibu/PlayniteExtensions, `source/EaLibrary/Services/EaWebsite.cs`).
//!
//! Login: the user signs in on www.ea.com. EA's site sends the browser through
//! accounts.ea.com and signin.ea.com and back to `https://www.ea.com/login_check?code=…`;
//! by then accounts.ea.com has set its session cookies (`sid`, `remid`), which is all we keep.
//!
//! Library: every refresh trades those cookies for a fresh, short-lived access token
//! (`accounts.ea.com/connect/auth?client_id=ORIGIN_JS_SDK&…&prompt=none`), lists the owned
//! games with the GraphQL query the EA website uses (`getPreloadedOwnedGames`), then looks up
//! each offer's content id in EA's catalog (`legacyOffers`).
//!
//! [`OwnedGame::id`] is the offer id (`Origin.OFR.50.0002487`, `OFB-EAST:52735`, `DR:225064100`).
//! `extra` keys:
//! - `offer_id`: same as `id`.
//! - `content_id`: catalog content id. The EA app's install/launch link takes it:
//!   `origin2://game/launch/?offerIds=<content_id>` (EaLibrary `EaControllerHelper.cs`).
//! - `slug`: the game's slug on ea.com (`a-way-out`).
//! - `product_id`: EA's product id for this edition.
//! - `ownership_methods`: EA's ownership methods, comma-separated (`PURCHASE,ENTITLEMENT_GRANT`).
//! - `access`: how the account has the game, one of `ea` (bought on EA), `steam` or `epic`
//!   (bought there and linked to the EA account), `ea_play` or `game_pass` (subscription).

use std::collections::HashMap;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    http::{self, cookie_jar},
    Cookie, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId,
};

const LOGIN_URL: &str = "https://www.ea.com/login";
/// EA's site redirects here (with `?code=…`) once accounts.ea.com accepted the sign-in.
const LOGIN_DONE_PREFIX: &str = "https://www.ea.com/login_check";
const COOKIE_URLS: [&str; 3] = [
    "https://accounts.ea.com/",
    "https://signin.ea.com/",
    "https://www.ea.com/",
];
const COOKIE_DOMAIN: &str = "ea.com";
/// The host whose cookies carry the EA sign-in; refreshed cookies are read back from it.
const ACCOUNTS_HOST: &str = "accounts.ea.com";
const TOKEN_URL: &str = "https://accounts.ea.com/connect/auth?client_id=ORIGIN_JS_SDK&response_type=token&redirect_uri=nucleus:rest&prompt=none";
const GRAPHQL_URL: &str = "https://service-aggregation-layer.juno.ea.com/graphql";
const OWNED_GAMES_QUERY_HASH: &str =
    "779f1cd1355699752e20c0b3877847f4e3010ef5de131c248e98f8eff84f0718";
const PAGE_SIZE: u32 = 500;
const MAX_PAGES: usize = 50;
const LEGACY_OFFER_BATCH: usize = 100;

const REAL_OWNERSHIP: [&str; 8] = [
    "UNKNOWN",
    "ASSOCIATION",
    "PURCHASE",
    "REDEMPTION",
    "GIFT_RECEIPT",
    "ENTITLEMENT_GRANT",
    "DIRECT_ENTITLEMENT",
    "PRE_ORDER_PURCHASE",
];
const EA_PLAY_OWNERSHIP: [&str; 5] = [
    "VAULT",
    "STEAM_VAULT",
    "STEAM_SUBSCRIPTION",
    "EPIC_VAULT",
    "EPIC_SUBSCRIPTION",
];

/// What the session file holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct EaSession {
    cookies: Vec<Cookie>,
}

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::Ea,
        start_url: LOGIN_URL.to_string(),
        done_url_prefixes: vec![LOGIN_DONE_PREFIX.to_string()],
        cookie_urls: COOKIE_URLS.iter().map(|u| u.to_string()).collect(),
        // The sign-in lives in HttpOnly cookies, which a user cannot reasonably copy.
        manual: ManualLogin::Unsupported,
        user_agent: None,
        state: serde_json::Value::Null,
    }
}

pub async fn complete_login(
    _client: &Client,
    _pending: &PendingLogin,
    capture: &LoginCapture,
) -> eyre::Result<Session> {
    let mut cookies = cookie_jar::keep_domain(&capture.cookies, COOKIE_DOMAIN);
    if cookies.is_empty() {
        return Err(eyre::format_err!(
            "The EA sign-in window returned no ea.com cookies. Sign in again."
        ));
    }
    let (client, jar) = cookie_jar::client_with(&cookies)?;
    let token = access_token(&client)
        .await
        .map_err(|err| eyre::format_err!("Signing in to EA did not finish: {err}"))?;
    let account_name = player_name(&client, &token).await;
    cookie_jar::refresh(&mut cookies, &jar, &[ACCOUNTS_HOST]);
    Ok(Session {
        account_name,
        data: serde_json::to_value(EaSession { cookies })?,
    })
}

pub async fn fetch_owned(_client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut data: EaSession = serde_json::from_value(session.data.clone()).map_err(|err| {
        eyre::format_err!("The saved EA login is unreadable ({err}). Sign in again.")
    })?;
    let (client, jar) = cookie_jar::client_with(&data.cookies)?;
    let token = access_token(&client).await;
    // EA may have renewed `sid`; keep the renewed cookies even when something below fails.
    cookie_jar::refresh(&mut data.cookies, &jar, &[ACCOUNTS_HOST]);
    session.data = serde_json::to_value(&data)?;
    let token = token?;

    if session.account_name.is_none() {
        session.account_name = player_name(&client, &token).await;
    }

    let mut games = Vec::new();
    let mut cursor = "0".to_string();
    for _ in 0..MAX_PAGES {
        let body = graphql_get(&client, &token, &owned_games_url(&cursor)?).await?;
        let page = parse_owned_games_page(&body)?;
        games.extend(page.games);
        match page.next {
            Some(next) if !next.is_empty() && next != cursor => cursor = next,
            _ => break,
        }
    }

    // Content ids only improve the install link and matching installed copies; without
    // them the offer id is used, so a failed lookup never fails the library.
    let offer_ids: Vec<String> = games.iter().map(|g| g.id.clone()).collect();
    let mut content_ids = HashMap::new();
    for batch in offer_ids.chunks(LEGACY_OFFER_BATCH) {
        match legacy_content_ids(&client, batch).await {
            Ok(found) => content_ids.extend(found),
            Err(err) => eprintln!("Could not look up EA content ids, using offer ids: {err:?}"),
        }
    }
    Ok(games
        .into_iter()
        .map(|game| match content_ids.get(&game.id) {
            Some(content_id) => game.with_extra("content_id", content_id.clone()),
            None => game,
        })
        .collect())
}

/// The ids in the `id=` value of an installed EA game's `.mfst` manifest (in
/// `ProgramData/Origin/LocalContent`), as BoilR's installed EA platform reads it: URL-encoded,
/// possibly several offer ids joined by commas (`OFB-EAST%3a52735`,
/// `Origin.OFR.50.0002487%2cOrigin.OFR.50.0002412`). The account lists the game under one of
/// these, or under its content id.
pub fn manifest_ids(manifest_id: &str) -> Vec<String> {
    let decoded = url::form_urlencoded::parse(format!("id={manifest_id}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| manifest_id.to_string());
    decoded
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

async fn access_token(client: &Client) -> eyre::Result<String> {
    let response = client
        .get(TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?;
    let body = response.text().await?;
    parse_token_response(&body)
}

/// The display name of the signed-in account, if EA hands it out. Never fails a login.
async fn player_name(client: &Client, token: &str) -> Option<String> {
    let body = serde_json::json!({ "query": "query { me { player { displayName } } }" });
    let response = client
        .post(GRAPHQL_URL)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .ok()?;
    parse_player_name(&response.text().await.ok()?)
}

/// GETs a GraphQL URL. EA answers a rejected token with 401 and a GraphQL error body, which
/// the parsers turn into a readable error, so the body is returned whatever the status.
async fn graphql_get(client: &Client, token: &str, url: &str) -> eyre::Result<String> {
    let response = client.get(url).bearer_auth(token).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() && serde_json::from_str::<serde_json::Value>(&body).is_err() {
        return Err(eyre::format_err!(
            "{status} from EA's library service: {}",
            http::snippet(&body)
        ));
    }
    Ok(body)
}

async fn legacy_content_ids(
    client: &Client,
    offer_ids: &[String],
) -> eyre::Result<HashMap<String, String>> {
    // The same query EaLibrary sends (getLegacyCatalogDefs), cut down to the fields we use.
    let body = serde_json::json!({
        "query": "query getLegacyCatalogDefs($offerIds: [String!]!, $locale: Locale) { legacyOffers(offerIds: $offerIds, locale: $locale) { offerId: id contentId } }",
        "operationName": "getLegacyCatalogDefs",
        "variables": { "locale": "DEFAULT", "offerIds": offer_ids },
    });
    let response = http::send(client.post(GRAPHQL_URL).json(&body)).await?;
    parse_legacy_offers(&response.text().await?)
}

fn owned_games_url(cursor: &str) -> eyre::Result<String> {
    // The variables and persisted-query hash EaLibrary uses (EaWebsite.GetGamesUrl).
    let variables = serde_json::json!({
        "isMac": false,
        "addFieldsToPreloadGames": true,
        "locale": "en",
        "limit": PAGE_SIZE,
        "next": cursor,
        "type": ["DIGITAL_FULL_GAME", "PACKAGED_FULL_GAME"],
        "entitlementEnabled": true,
        "storefronts": ["EA", "STEAM", "EPIC"],
        "ownershipMethods": [
            "UNKNOWN", "ASSOCIATION", "PURCHASE", "REDEMPTION", "GIFT_RECEIPT",
            "ENTITLEMENT_GRANT", "DIRECT_ENTITLEMENT", "PRE_ORDER_PURCHASE", "VAULT",
            "XGP_VAULT", "STEAM", "STEAM_VAULT", "STEAM_SUBSCRIPTION", "EPIC", "EPIC_VAULT",
            "EPIC_SUBSCRIPTION"
        ],
        "platforms": ["PC"],
    });
    let extensions = serde_json::json!({
        "persistedQuery": { "version": 1, "sha256Hash": OWNED_GAMES_QUERY_HASH }
    });
    let mut url = url::Url::parse(GRAPHQL_URL)?;
    url.query_pairs_mut()
        .append_pair("operationName", "getPreloadedOwnedGames")
        .append_pair("variables", &variables.to_string())
        .append_pair("extensions", &extensions.to_string());
    Ok(url.into())
}

// ---- Parsing (pure) ----

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

fn parse_token_response(body: &str) -> eyre::Result<String> {
    let parsed: TokenResponse = serde_json::from_str(body).map_err(|_| {
        eyre::format_err!(
            "EA did not hand out an access token; the saved sign-in has probably expired. Sign in again. EA answered: {}",
            http::snippet(body)
        )
    })?;
    match (parsed.access_token, parsed.error) {
        (Some(token), _) if !token.is_empty() => Ok(token),
        (_, Some(error)) if error == "login_required" => Err(eyre::format_err!(
            "Your EA sign-in has expired. Sign in again."
        )),
        (_, Some(error)) => Err(eyre::format_err!(
            "EA refused the saved sign-in ({error}). Sign in again."
        )),
        _ => Err(eyre::format_err!(
            "EA did not hand out an access token. Sign in again. EA answered: {}",
            http::snippet(body)
        )),
    }
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: Option<String>,
    extensions: Option<GraphQlErrorExtensions>,
}

#[derive(Debug, Deserialize)]
struct GraphQlErrorExtensions {
    code: Option<String>,
}

impl<T> GraphQlResponse<T> {
    /// The data, or a readable error built from `errors`. Partial data (data plus errors on
    /// some fields) is kept, unless EA says the sign-in is not valid.
    fn into_data(self, what: &str) -> eyre::Result<T> {
        let errors = self.errors.unwrap_or_default();
        if errors.iter().any(|e| {
            e.extensions
                .as_ref()
                .and_then(|x| x.code.as_deref())
                .is_some_and(|code| code == "UNAUTHENTICATED")
        }) {
            return Err(eyre::format_err!(
                "EA rejected the saved sign-in while listing {what}. Sign in again."
            ));
        }
        match self.data {
            Some(data) => Ok(data),
            None => {
                let messages: Vec<&str> =
                    errors.iter().filter_map(|e| e.message.as_deref()).collect();
                Err(eyre::format_err!(
                    "EA could not list {what}: {}",
                    if messages.is_empty() {
                        "no data in the answer".to_string()
                    } else {
                        messages.join("; ")
                    }
                ))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct OwnedGamesData {
    me: Option<OwnedGamesUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnedGamesUser {
    owned_game_products: Option<OwnedGamesPage>,
}

#[derive(Debug, Deserialize)]
struct OwnedGamesPage {
    next: Option<String>,
    items: Option<Vec<OwnedGameProduct>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnedGameProduct {
    id: Option<String>,
    origin_offer_id: Option<String>,
    product: Option<Product>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    id: Option<String>,
    name: Option<String>,
    game_slug: Option<String>,
    base_item: Option<BaseItem>,
    game_product_user: Option<GameProductUser>,
}

#[derive(Debug, Deserialize)]
struct BaseItem {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameProductUser {
    ownership_methods: Option<Vec<String>>,
}

#[derive(Debug)]
struct ParsedPage {
    games: Vec<OwnedGame>,
    next: Option<String>,
}

fn parse_owned_games_page(body: &str) -> eyre::Result<ParsedPage> {
    let response: GraphQlResponse<OwnedGamesData> = serde_json::from_str(body).map_err(|err| {
        eyre::format_err!(
            "Unexpected answer from EA's library service: {err}. Body starts with: {}",
            http::snippet(body)
        )
    })?;
    let page = response
        .into_data("your games")?
        .me
        .and_then(|me| me.owned_game_products)
        .ok_or_else(|| eyre::format_err!("EA returned no game list for this account"))?;
    Ok(ParsedPage {
        games: page
            .items
            .unwrap_or_default()
            .into_iter()
            .filter_map(to_owned_game)
            .collect(),
        next: page.next,
    })
}

fn to_owned_game(item: OwnedGameProduct) -> Option<OwnedGame> {
    let offer_id = item
        .origin_offer_id
        .filter(|id| !id.is_empty())
        .or(item.id)
        .filter(|id| !id.is_empty())?;
    let product = item.product?;
    let title = product
        .name
        .as_deref()
        .map(clean_title)
        .filter(|t| !t.is_empty())
        .or_else(|| {
            product
                .base_item
                .as_ref()
                .and_then(|b| b.title.as_deref())
                .map(clean_title)
        })
        .filter(|t| !t.is_empty())?;
    let methods = product
        .game_product_user
        .and_then(|u| u.ownership_methods)
        .unwrap_or_default();
    let mut game = OwnedGame::new(StoreId::Ea, offer_id.clone(), title)
        .with_extra("offer_id", offer_id)
        .with_extra("access", access(&methods))
        .with_extra("ownership_methods", methods.join(","));
    if let Some(slug) = product.game_slug.filter(|s| !s.is_empty()) {
        game = game.with_extra("slug", slug);
    }
    if let Some(product_id) = product.id.filter(|s| !s.is_empty()) {
        game = game.with_extra("product_id", product_id);
    }
    Some(game)
}

/// How the account has a game, from EA's ownership methods. A purchase on another store wins
/// over an EA entitlement (Steam purchases also carry `ENTITLEMENT_GRANT`), and owning wins
/// over a subscription.
fn access(methods: &[String]) -> &'static str {
    let has = |m: &str| methods.iter().any(|x| x == m);
    if has("STEAM") {
        "steam"
    } else if has("EPIC") {
        "epic"
    } else if REAL_OWNERSHIP.iter().any(|m| has(m)) {
        "ea"
    } else if has("XGP_VAULT") {
        "game_pass"
    } else if EA_PLAY_OWNERSHIP.iter().any(|m| has(m)) {
        "ea_play"
    } else {
        "ea"
    }
}

/// Drops trademark signs, which EA puts in most names and which hurt artwork searches.
fn clean_title(name: &str) -> String {
    name.replace(['™', '®', '©'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyOffersData {
    legacy_offers: Option<Vec<Option<LegacyOffer>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyOffer {
    offer_id: Option<String>,
    content_id: Option<String>,
}

/// Offer id to content id.
fn parse_legacy_offers(body: &str) -> eyre::Result<HashMap<String, String>> {
    let response: GraphQlResponse<LegacyOffersData> =
        serde_json::from_str(body).map_err(|err| {
            eyre::format_err!(
                "Unexpected answer from EA's catalog: {err}. Body starts with: {}",
                http::snippet(body)
            )
        })?;
    Ok(response
        .into_data("game details")?
        .legacy_offers
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter_map(|o| match (o.offer_id, o.content_id) {
            (Some(offer), Some(content)) if !offer.is_empty() && !content.is_empty() => {
                Some((offer, content))
            }
            _ => None,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct PlayerData {
    me: Option<PlayerUser>,
}

#[derive(Debug, Deserialize)]
struct PlayerUser {
    player: Option<Player>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Player {
    display_name: Option<String>,
}

fn parse_player_name(body: &str) -> Option<String> {
    serde_json::from_str::<GraphQlResponse<PlayerData>>(body)
        .ok()?
        .data?
        .me?
        .player?
        .display_name
        .filter(|n| !n.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/testdata/ea/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|err| format!("missing {path}: {err}"))
    }

    fn cookie(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
        }
    }

    #[test]
    fn login_urls_do_not_finish_early() {
        let pending = begin_login();
        let done = |url: &str| {
            pending
                .done_url_prefixes
                .iter()
                .any(|prefix| url.starts_with(prefix.as_str()))
        };
        assert!(!done(&pending.start_url));
        // The pages EA's sign-in goes through (seen 2026-11-08).
        assert!(!done("https://accounts.ea.com/connect/auth?response_type=code&redirect_uri=https%3A%2F%2Fwww.ea.com%2Flogin_check&state=x&locale=en_US&client_id=EADOTCOM-WEB-SERVER&display=junoWeb%2Flogin"));
        assert!(!done("https://signin.ea.com/p/juno/login?fid=abc"));
        assert!(done("https://www.ea.com/login_check?code=QUOx&state=x"));
    }

    #[test]
    fn reads_access_token() -> eyre::Result<()> {
        assert_eq!(
            parse_token_response(&fixture("token_ok.json"))?,
            "QVQwOjIuMDozLjA6NjA6dGVzdHRva2Vu"
        );
        Ok(())
    }

    #[test]
    fn expired_cookies_ask_to_sign_in_again() {
        let err = parse_token_response(&fixture("token_login_required.json"))
            .map(|_| ())
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("Sign in again"), "{err}");

        let err = parse_token_response("<html>login</html>")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("Sign in again"), "{err}");
    }

    #[test]
    fn parses_owned_games_pages() -> eyre::Result<()> {
        let first = parse_owned_games_page(&fixture("owned_games_page1.json"))?;
        assert_eq!(first.next.as_deref(), Some("Mw=="));
        assert_eq!(first.games.len(), 3);

        let way_out = first
            .games
            .first()
            .ok_or_else(|| eyre::format_err!("empty"))?;
        assert_eq!(way_out.store, StoreId::Ea);
        assert_eq!(way_out.id, "Origin.OFR.50.0002487");
        assert_eq!(way_out.title, "A Way Out");
        assert_eq!(way_out.extra("offer_id"), Some("Origin.OFR.50.0002487"));
        assert_eq!(way_out.extra("slug"), Some("a-way-out"));
        assert_eq!(way_out.extra("access"), Some("steam"));
        assert_eq!(way_out.extra("ownership_methods"), Some("STEAM"));
        assert_eq!(
            way_out.extra("product_id"),
            Some("en_a-way-out-pc-jp-juno_VideoGameProduct_nl_pc")
        );

        let accesses: Vec<_> = first
            .games
            .iter()
            .filter_map(|g| g.extra("access"))
            .collect();
        assert_eq!(accesses, vec!["steam", "ea", "game_pass"]);

        let second = parse_owned_games_page(&fixture("owned_games_page2.json"))?;
        assert_eq!(second.next, None);
        let zuma = second
            .games
            .first()
            .ok_or_else(|| eyre::format_err!("empty"))?;
        assert_eq!(zuma.id, "OFB-EAST:52735");
        // Trademark signs are dropped.
        assert_eq!(zuma.title, "Zuma's Revenge");
        Ok(())
    }

    #[test]
    fn unauthenticated_library_asks_to_sign_in_again() {
        let err = parse_owned_games_page(&fixture("not_authenticated.json"))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("Sign in again"), "{err}");
    }

    #[test]
    fn other_graphql_errors_are_reported() {
        let err = parse_owned_games_page(
            r#"{"data":null,"errors":[{"message":"PersistedQueryNotFound"}]}"#,
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(err.contains("PersistedQueryNotFound"), "{err}");
    }

    #[test]
    fn parses_content_ids() -> eyre::Result<()> {
        let ids = parse_legacy_offers(&fixture("legacy_offers.json"))?;
        assert_eq!(ids.len(), 4);
        assert_eq!(
            ids.get("Origin.OFR.50.0002487").map(String::as_str),
            Some("1028663")
        );
        Ok(())
    }

    #[test]
    fn parses_player_name() {
        assert_eq!(
            parse_player_name(&fixture("player.json")),
            Some("PlayerOne".to_string())
        );
        assert_eq!(parse_player_name(&fixture("not_authenticated.json")), None);
    }

    #[test]
    fn owned_games_url_carries_cursor_and_query_hash() -> eyre::Result<()> {
        let url = owned_games_url("Mw==")?;
        assert_eq!(
            http::query_param(&url, "operationName").as_deref(),
            Some("getPreloadedOwnedGames")
        );
        let variables: serde_json::Value =
            serde_json::from_str(&http::query_param(&url, "variables").unwrap_or_default())?;
        assert_eq!(variables.get("next"), Some(&serde_json::json!("Mw==")));
        assert!(http::query_param(&url, "extensions")
            .unwrap_or_default()
            .contains(OWNED_GAMES_QUERY_HASH));
        Ok(())
    }

    #[test]
    fn reads_installed_manifest_ids() {
        assert_eq!(manifest_ids("OFB-EAST%3a52735"), vec!["OFB-EAST:52735"]);
        assert_eq!(
            manifest_ids("Origin.OFR.50.0000001%2cOFB-EAST%3a52735"),
            vec!["Origin.OFR.50.0000001", "OFB-EAST:52735"]
        );
        assert!(manifest_ids("").is_empty());
    }

    #[test]
    fn cookie_jar_round_trips_and_refreshes() -> eyre::Result<()> {
        let captured = vec![
            cookie("sid", "old", "accounts.ea.com"),
            cookie("remid", "r", ".ea.com"),
            cookie("sid", "old", "accounts.ea.com"),
            cookie("tracker", "t", ".example.com"),
            cookie("www", "w", "www.ea.com"),
        ];
        let mut kept = cookie_jar::keep_domain(&captured, COOKIE_DOMAIN);
        assert_eq!(kept.len(), 3);

        let (_client, jar) = cookie_jar::client_with(&kept)?;
        let mut sent = cookie_jar::pairs_for(&jar, ACCOUNTS_HOST);
        sent.sort();
        assert_eq!(
            sent,
            vec![
                ("remid".to_string(), "r".to_string()),
                ("sid".to_string(), "old".to_string())
            ]
        );
        // Host-only cookies stay on their host.
        assert_eq!(
            cookie_jar::pairs_for(&jar, "signin.ea.com"),
            vec![("remid".to_string(), "r".to_string())]
        );

        // EA renews `sid` (host-only, as it was) and sets a new cookie.
        let url = url::Url::parse("https://accounts.ea.com/connect/auth")?;
        jar.add_cookie_str("sid=new; Path=/", &url);
        jar.add_cookie_str("_nx_mpcid=m; Path=/", &url);
        assert_eq!(cookie_jar::pairs_for(&jar, ACCOUNTS_HOST).len(), 3);
        cookie_jar::refresh(&mut kept, &jar, &[ACCOUNTS_HOST]);
        let value = |name: &str| {
            kept.iter()
                .find(|c| c.name == name)
                .map(|c| c.value.clone())
        };
        assert_eq!(value("sid").as_deref(), Some("new"));
        assert_eq!(value("_nx_mpcid").as_deref(), Some("m"));
        assert_eq!(value("www").as_deref(), Some("w"));
        assert_eq!(kept.len(), 4);
        Ok(())
    }
}
