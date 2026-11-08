//! Battle.net. Follows Playnite's BattleNetLibrary
//! (github.com/JosefNemec/PlayniteExtensions, `source/Libraries/BattleNetLibrary`).
//!
//! Login: the window starts the account site's OAuth sign-in
//! (`account.battle.net/oauth2/authorization/account-settings`). After the user signs in,
//! oauth.battle.net redirects to `account.battle.net/callback/oauth2/code/account-settings`;
//! by then the long-lived Battle.net sign-in cookies (on `.battle.net`) are set, and those are
//! what we keep. The account site's own session lasts 30 minutes, so every refresh replays
//! that OAuth sign-in with the kept cookies (as Playnite does) before calling the account API.
//!
//! Library: `account.battle.net/api/games-and-subs` lists game accounts by Blizzard title id;
//! [`PRODUCTS`] maps them to launcher product codes. `api/classic-games` lists the CD-key
//! classics (Warcraft III, Diablo II). Titles not in [`PRODUCTS`] are skipped: without a
//! product code the launcher cannot open them.
//!
//! [`OwnedGame::id`] is the launcher product code (`Pro`, `WoW`, `WTCG`), the code
//! `Battle.net.exe --exec="launch <code>"` takes. `extra` keys:
//! - `internal_id`: the launcher's internal id (`prometheus`), taken by
//!   `Battle.net.exe --game=<internal_id>` (opens the game's page, with its Install button).
//!   Absent for classic games.
//! - `title_id`: Blizzard's numeric title id from the account API. Absent for classic games.
//! - `launcher`: `true` when the Battle.net launcher installs and starts the game, `false` for
//!   classic games, which are installed from Blizzard's site with a CD key.

use reqwest::{cookie::Jar, Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    ea::cookie_jar, http, Cookie, LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session,
    StoreId,
};

/// Starts the account site's OAuth sign-in. Also replayed on every refresh.
const SIGN_IN_URL: &str = "https://account.battle.net/oauth2/authorization/account-settings";
/// Where oauth.battle.net sends the browser once the user is signed in.
const LOGIN_DONE_PREFIX: &str = "https://account.battle.net/callback/oauth2/code/";
const COOKIE_URLS: [&str; 7] = [
    "https://account.battle.net/",
    "https://oauth.battle.net/",
    "https://battle.net/",
    "https://us.account.battle.net/",
    "https://eu.account.battle.net/",
    "https://kr.account.battle.net/",
    "https://tw.account.battle.net/",
];
const COOKIE_DOMAIN: &str = "battle.net";
const ACCOUNT_HOST: &str = "account.battle.net";
const OAUTH_HOST: &str = "oauth.battle.net";
const API_STATUS_URL: &str = "https://account.battle.net/api/";
const GAMES_URL: &str = "https://account.battle.net/api/games-and-subs";
const CLASSIC_GAMES_URL: &str = "https://account.battle.net/api/classic-games";
const DETAILS_URL: &str = "https://account.battle.net/api/details";

/// A game the Battle.net account API can return. Ported from Playnite's `BattleNetGames.cs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    /// Launcher product code (`--exec="launch <code>"`).
    pub code: &'static str,
    /// Launcher internal id (`--game=<internal_id>`); for classic games, the name Blizzard's
    /// installer registers in Windows' uninstall list.
    pub internal_id: &'static str,
    pub name: &'static str,
    /// Blizzard title id in `api/games-and-subs`. `None` for classic games.
    pub title_id: Option<u64>,
    /// Whether the Battle.net launcher installs and starts it (false: classic CD-key game).
    pub launcher: bool,
}

const fn launcher(
    code: &'static str,
    internal_id: &'static str,
    name: &'static str,
    title_id: u64,
) -> Product {
    Product {
        code,
        internal_id,
        name,
        title_id: Some(title_id),
        launcher: true,
    }
}

const fn classic(code: &'static str, internal_id: &'static str, name: &'static str) -> Product {
    Product {
        code,
        internal_id,
        name,
        title_id: None,
        launcher: false,
    }
}

/// Order matters where title ids repeat: the first entry wins (as in Playnite).
pub const PRODUCTS: &[Product] = &[
    launcher("WoW", "wow", "World of Warcraft", 5730135),
    launcher("D3", "diablo3", "Diablo III", 17459),
    launcher("S2", "s2", "StarCraft II", 21298),
    launcher("S1", "s1", "StarCraft", 21297),
    launcher("WTCG", "hs_beta", "Hearthstone", 1465140039),
    launcher("Hero", "heroes", "Heroes of the Storm", 1214607983),
    launcher("Pro", "prometheus", "Overwatch 2", 5272175),
    classic("D2", "Diablo II", "Diablo II"),
    classic("D2X", "Diablo II", "Diablo II: Lord of Destruction"),
    launcher("VIPR", "viper", "Call of Duty: Black Ops 4", 1447645266),
    launcher("ODIN", "odin", "Call of Duty: Modern Warfare", 1329875278),
    classic("W3C", "Warcraft III", "Warcraft III: Reign of Chaos"),
    classic("W3CX", "Warcraft III", "Warcraft III: The Frozen Throne"),
    launcher("W3", "w3", "Warcraft III: Reforged", 22323),
    launcher(
        "LAZR",
        "lazarus",
        "Call of Duty: Modern Warfare 2 Campaign Remastered",
        1279351378,
    ),
    launcher(
        "ZEUS",
        "zeus",
        "Call of Duty: Black Ops Cold War",
        1514493267,
    ),
    launcher("WLBY", "wlby", "Crash Bandicoot 4", 1464615513),
    launcher("OSI", "osi", "Diablo II: Resurrected", 5198665),
    launcher("RTRO", "rtro", "Blizzard Arcade Collection", 1381257807),
    launcher("FORE", "fore", "Call of Duty: Vanguard", 1179603525),
    launcher("ANBS", "anbs", "Diablo Immortal", 1095647827),
    // Playnite lists Modern Warfare III (PNTA, "pinta") under the same title id; both
    // live in the Call of Duty launcher entry, so the account API cannot tell them apart.
    launcher(
        "AUKS",
        "auks",
        "Call of Duty: Modern Warfare II",
        1096108883,
    ),
    launcher("Fen", "Fen", "Diablo IV", 4613486),
    launcher("D1", "D1", "Diablo", 1146246220),
    launcher("W1R", "w1r", "Warcraft: Remastered", 5714258),
    launcher("W2R", "w2r", "Warcraft II: Remastered", 5714514),
    launcher("W1", "w1", "Warcraft: Orcs & Humans", 1463898673),
    launcher("W2", "w2", "Warcraft II: Battle.net Edition", 1462911566),
    launcher("GRY", "gryphon", "Warcraft Rumble", 4674137),
    launcher("ARIS", "aris", "Doom: The Dark Ages", 1095911763),
    launcher("SCOR", "scorpio", "Sea of Thieves", 1396920146),
    launcher("ARK", "arkansas", "The Outer Worlds 2", 4280907),
    launcher("LBRA", "libra", "Tony Hawk's Pro Skater 3 + 4", 1279414849),
    launcher(
        "PNTA",
        "pinta",
        "Call of Duty: Modern Warfare III",
        1096108883,
    ),
    launcher("AQUA", "aqua", "Avowed", 1095849281),
];

/// The product with this launcher code (case-insensitive).
pub fn product(code: &str) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.code.eq_ignore_ascii_case(code))
}

fn product_for_title(title_id: u64) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.title_id == Some(title_id))
}

/// What the session file holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BattleNetSession {
    cookies: Vec<Cookie>,
}

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::BattleNet,
        start_url: SIGN_IN_URL.to_string(),
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
            "The Battle.net sign-in window returned no battle.net cookies. Sign in again."
        ));
    }
    let (client, jar) = cookie_jar::client_with(&cookies)?;
    sign_in_account_site(&client)
        .await
        .map_err(|err| eyre::format_err!("Signing in to Battle.net did not finish: {err}"))?;
    let account_name = battletag(&client, &jar).await;
    cookie_jar::refresh(&mut cookies, &jar, &[OAUTH_HOST]);
    Ok(Session {
        account_name,
        data: serde_json::to_value(BattleNetSession { cookies })?,
    })
}

pub async fn fetch_owned(_client: &Client, session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    let mut data: BattleNetSession =
        serde_json::from_value(session.data.clone()).map_err(|err| {
            eyre::format_err!("The saved Battle.net login is unreadable ({err}). Sign in again.")
        })?;
    let (client, jar) = cookie_jar::client_with(&data.cookies)?;
    let signed_in = sign_in_account_site(&client).await;
    // Battle.net may have renewed its sign-in cookies; keep them even when something fails.
    cookie_jar::refresh(&mut data.cookies, &jar, &[OAUTH_HOST]);
    session.data = serde_json::to_value(&data)?;
    signed_in?;

    if let Some(tag) = battletag(&client, &jar).await {
        session.account_name = Some(tag);
    }
    let games: GamesAndSubs = api_get(&client, &jar, GAMES_URL).await?;
    let classic: ClassicGames = api_get(&client, &jar, CLASSIC_GAMES_URL).await?;
    Ok(owned_games(&games, &classic))
}

/// Replays the account site's OAuth sign-in with the saved cookies, which gives the client a
/// fresh account-site session, then checks that the account API sees a signed-in user.
async fn sign_in_account_site(client: &Client) -> eyre::Result<()> {
    // Ends on the account site when the saved sign-in is still good, on the login page when
    // not; either way it answers 200, so the status check below decides.
    client.get(SIGN_IN_URL).send().await?;
    let response = client
        .get(API_STATUS_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    let parsed = parse_api_status(&body);
    match parsed {
        Some(api) if api.authenticated => Ok(()),
        Some(_) => Err(eyre::format_err!(
            "Your Battle.net sign-in has expired. Sign in again."
        )),
        None if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN => Err(
            eyre::format_err!("Your Battle.net sign-in has expired. Sign in again."),
        ),
        None => Err(eyre::format_err!(
            "Unexpected answer from the Battle.net account site ({status}): {}",
            http::snippet(&body)
        )),
    }
}

/// The account's BattleTag, if the account site hands it out. Never fails a refresh.
async fn battletag(client: &Client, jar: &Jar) -> Option<String> {
    let details: AccountDetails = api_get(client, jar, DETAILS_URL).await.ok()?;
    details.battle_tag.filter(|tag| !tag.trim().is_empty())
}

/// A GET on the account API, with the headers its own web page sends.
async fn api_get<T: DeserializeOwned>(client: &Client, jar: &Jar, url: &str) -> eyre::Result<T> {
    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    if let Some((_, xsrf)) = cookie_jar::pairs_for(jar, ACCOUNT_HOST)
        .into_iter()
        .find(|(name, _)| name == "XSRF-TOKEN")
    {
        request = request.header("X-XSRF-TOKEN", xsrf);
    }
    http::send_json(request).await
}

// ---- Parsing (pure) ----

#[derive(Debug, Deserialize)]
struct ApiStatus {
    #[serde(default)]
    authenticated: bool,
}

fn parse_api_status(body: &str) -> Option<ApiStatus> {
    serde_json::from_str(body).ok()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountDetails {
    battle_tag: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GamesAndSubs {
    game_accounts: Option<Vec<GameAccount>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameAccount {
    title_id: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClassicGames {
    classic_games: Option<Vec<ClassicGame>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClassicGame {
    regional_game_franchise_icon_filename: Option<String>,
}

fn owned_games(games: &GamesAndSubs, classic: &ClassicGames) -> Vec<OwnedGame> {
    let mut products: Vec<&'static Product> = Vec::new();
    let mut add = |product: Option<&'static Product>| {
        if let Some(product) = product {
            // Several game accounts (two WoW licences, say) are one game.
            if !products.iter().any(|p| p.code == product.code) {
                products.push(product);
            }
        }
    };

    for account in games.game_accounts.iter().flatten() {
        add(account.title_id.and_then(product_for_title));
    }

    // As Playnite: classic games only show up as CD keys, told apart by their icon. One
    // Warcraft III key is Reign of Chaos, a second one The Frozen Throne; likewise Diablo II
    // and Lord of Destruction.
    let icons: Vec<&str> = classic
        .classic_games
        .iter()
        .flatten()
        .filter_map(|g| g.regional_game_franchise_icon_filename.as_deref())
        .collect();
    let count = |franchise: &str| {
        icons
            .iter()
            .filter(|icon| is_franchise(icon, franchise))
            .count()
    };
    for (franchise, base, expansion) in
        [("warcraft-iii", "W3C", "W3CX"), ("diablo-ii", "D2", "D2X")]
    {
        let keys = count(franchise);
        if keys >= 1 {
            add(product(base));
        }
        if keys >= 2 {
            add(product(expansion));
        }
    }

    products.into_iter().map(to_owned_game).collect()
}

/// Whether an icon file name belongs to a franchise, without `diablo-ii` matching
/// `diablo-iii`.
fn is_franchise(icon: &str, franchise: &str) -> bool {
    let icon = icon.to_ascii_lowercase();
    icon.match_indices(franchise).any(|(start, _)| {
        !icon
            .get(start + franchise.len()..)
            .is_some_and(|rest| rest.starts_with('i'))
    })
}

fn to_owned_game(product: &Product) -> OwnedGame {
    let mut game = OwnedGame::new(StoreId::BattleNet, product.code, product.name)
        .with_extra("launcher", product.launcher.to_string());
    if product.launcher {
        game = game.with_extra("internal_id", product.internal_id);
    }
    if let Some(title_id) = product.title_id {
        game = game.with_extra("title_id", title_id.to_string());
    }
    game
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture<T: DeserializeOwned>(name: &str) -> eyre::Result<T> {
        let path = format!("{}/testdata/battlenet/{name}", env!("CARGO_MANIFEST_DIR"));
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    fn fixture_text(name: &str) -> eyre::Result<String> {
        let path = format!("{}/testdata/battlenet/{name}", env!("CARGO_MANIFEST_DIR"));
        Ok(std::fs::read_to_string(path)?)
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
        // The pages the sign-in goes through (seen 2026-11-08).
        assert!(!done("https://oauth.battle.net/authorize?response_type=code&client_id=x&redirect_uri=https://account.battle.net/callback/oauth2/code/account-settings"));
        assert!(!done("https://eu.account.battle.net/login/en-us/?ref=https://oauth.battle.net/authorize?redirect_uri%3Dhttps://account.battle.net/callback/oauth2/code/account-settings&app=bas"));
        assert!(!done("https://account.battle.net/overview"));
        assert!(done(
            "https://account.battle.net/callback/oauth2/code/account-settings?code=abc&state=x"
        ));
    }

    #[test]
    fn maps_game_accounts_and_classic_keys_to_products() -> eyre::Result<()> {
        let games: GamesAndSubs = fixture("games_and_subs.json")?;
        let classic: ClassicGames = fixture("classic_games.json")?;
        let owned = owned_games(&games, &classic);
        let codes: Vec<&str> = owned.iter().map(|g| g.id.as_str()).collect();
        // Two WoW accounts are one game; the unknown title is skipped; two Warcraft III keys
        // are the game and its expansion.
        assert_eq!(
            codes,
            vec!["WoW", "Pro", "WTCG", "Fen", "W3C", "W3CX", "D2"]
        );

        let overwatch = owned.get(1).ok_or_else(|| eyre::format_err!("missing"))?;
        assert_eq!(overwatch.store, StoreId::BattleNet);
        assert_eq!(overwatch.title, "Overwatch 2");
        assert_eq!(overwatch.extra("internal_id"), Some("prometheus"));
        assert_eq!(overwatch.extra("title_id"), Some("5272175"));
        assert_eq!(overwatch.extra("launcher"), Some("true"));

        let war3 = owned.get(4).ok_or_else(|| eyre::format_err!("missing"))?;
        assert_eq!(war3.title, "Warcraft III: Reign of Chaos");
        assert_eq!(war3.extra("launcher"), Some("false"));
        assert_eq!(war3.extra("internal_id"), None);
        assert_eq!(war3.extra("title_id"), None);
        Ok(())
    }

    #[test]
    fn empty_or_missing_lists_give_no_games() -> eyre::Result<()> {
        assert!(owned_games(&GamesAndSubs::default(), &ClassicGames::default()).is_empty());
        let null_lists: GamesAndSubs = serde_json::from_str(r#"{"gameAccounts":null}"#)?;
        let null_classic: ClassicGames = serde_json::from_str(r#"{"classicGames":null}"#)?;
        assert!(owned_games(&null_lists, &null_classic).is_empty());
        Ok(())
    }

    #[test]
    fn reads_sign_in_status() -> eyre::Result<()> {
        let anonymous = parse_api_status(&fixture_text("api_status_anonymous.json")?);
        assert!(anonymous.is_some_and(|s| !s.authenticated));
        let signed_in = parse_api_status(&fixture_text("api_status_authenticated.json")?);
        assert!(signed_in.is_some_and(|s| s.authenticated));
        assert!(parse_api_status("<!doctype html>").is_none());
        Ok(())
    }

    #[test]
    fn reads_battletag() -> eyre::Result<()> {
        let details: AccountDetails = fixture("details.json")?;
        assert_eq!(details.battle_tag.as_deref(), Some("Player#2187"));
        Ok(())
    }

    #[test]
    fn franchise_icons_do_not_overlap() {
        assert!(is_franchise("diablo-ii.png", "diablo-ii"));
        assert!(is_franchise("D2-Diablo-II-LoD.png", "diablo-ii"));
        assert!(!is_franchise("diablo-iii.png", "diablo-ii"));
        assert!(is_franchise(
            "warcraft-iii-frozen-throne.png",
            "warcraft-iii"
        ));
    }

    #[test]
    fn product_table_is_consistent() {
        for (i, p) in PRODUCTS.iter().enumerate() {
            assert!(
                !PRODUCTS.iter().skip(i + 1).any(|q| q.code == p.code),
                "duplicate code {}",
                p.code
            );
            assert_eq!(p.launcher, p.title_id.is_some(), "{}", p.code);
        }
        assert_eq!(product("pro").map(|p| p.internal_id), Some("prometheus"));
        assert_eq!(product_for_title(1096108883).map(|p| p.code), Some("AUKS"));
    }
}
