//! Store accounts: sign in to a store in a login window, then list every game the account
//! owns. The store logic lives in `boilr-stores`; this module runs the login window and
//! exposes the accounts to the UI.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use boilr::platforms::{
    accounts::{account_code_name, stores, InstallWith, StoreId},
    get_platforms,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use toml::Value;

use crate::{load_platform_table, persist_platform_table};

#[derive(Debug, Clone, Serialize)]
pub struct StoreAccount {
    #[serde(flatten)]
    status: stores::AccountStatus,
    code_name: String,
    enabled: bool,
    install_with: InstallWith,
}

#[derive(Debug, Clone, Serialize)]
struct LoginEvent {
    store: String,
    ok: bool,
    message: String,
}

fn parse_store(key: &str) -> Result<StoreId, String> {
    StoreId::from_key(key).ok_or_else(|| format!("Unknown store {key}"))
}

fn account(store: StoreId) -> StoreAccount {
    let code_name = account_code_name(store);
    let platform_settings = load_platform_table(code_name).unwrap_or_default();
    let enabled = platform_settings
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let install_with = platform_settings
        .get("install_with")
        .cloned()
        .and_then(|value| value.try_into::<InstallWith>().ok())
        .unwrap_or_default();
    StoreAccount {
        status: stores::account_status(store),
        code_name: code_name.to_string(),
        enabled,
        install_with,
    }
}

fn set_account_settings(
    store: StoreId,
    enabled: Option<bool>,
    install_with: Option<InstallWith>,
) -> Result<(), String> {
    let code_name = account_code_name(store);
    let mut table = load_platform_table(code_name)?;
    if let Some(enabled) = enabled {
        table.insert("enabled".to_string(), Value::Boolean(enabled));
    }
    if let Some(install_with) = install_with {
        let value = Value::try_from(install_with).map_err(|err| err.to_string())?;
        table.insert("install_with".to_string(), value);
    }
    persist_platform_table(code_name, table)
}

#[tauri::command(async)]
pub fn list_store_accounts() -> Vec<StoreAccount> {
    // Make sure every account section exists, so settings and sources stay in step.
    let _ = get_platforms();
    StoreId::ALL.into_iter().map(account).collect()
}

#[tauri::command(async)]
pub fn update_store_account(
    store: String,
    enabled: Option<bool>,
    install_with: Option<InstallWith>,
) -> Result<StoreAccount, String> {
    let store = parse_store(&store)?;
    set_account_settings(store, enabled, install_with)?;
    Ok(account(store))
}

#[tauri::command]
pub async fn refresh_store_library(store: String) -> Result<StoreAccount, String> {
    let store = parse_store(&store)?;
    stores::refresh_library(store)
        .await
        .map_err(|err| err.to_string())?;
    Ok(account(store))
}

#[tauri::command(async)]
pub fn sign_out_store(store: String) -> Result<StoreAccount, String> {
    let store = parse_store(&store)?;
    stores::delete_account(store).map_err(|err| err.to_string())?;
    Ok(account(store))
}

/// Opens the store's sign-in page for a manual login in the user's own browser.
#[tauri::command(async)]
pub fn open_store_sign_in_page(store: String) -> Result<(), String> {
    let store = parse_store(&store)?;
    let url = match stores::pending_login(store).manual {
        stores::ManualLogin::Paste { url, .. } => url,
        stores::ManualLogin::Unsupported => {
            return Err(format!("{} has no manual sign-in", store.display_name()))
        }
    };
    open_in_browser(&url).map_err(|err| format!("Could not open the browser: {err}"))
}

fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        command.arg(url);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}

/// Finishes a login with a value the user pasted (an API key, a code or a URL). Uses the
/// same pending login as the sign-in page the user opened, which Amazon needs (its PKCE
/// verifier and device serial are part of the sign-in link).
#[tauri::command]
pub async fn submit_store_login(store: String, pasted: String) -> Result<StoreAccount, String> {
    let store = parse_store(&store)?;
    let pending = stores::pending_login(store);
    let capture = stores::LoginCapture {
        pasted: Some(pasted.trim().to_string()),
        ..Default::default()
    };
    stores::complete_login(&pending, &capture)
        .await
        .map_err(|err| err.to_string())?;
    after_sign_in(store).await;
    Ok(account(store))
}

/// A first library fetch, and the account turned on as a game source.
async fn after_sign_in(store: StoreId) {
    if let Err(err) = set_account_settings(store, Some(true), None) {
        eprintln!("Could not turn on the {} account: {err}", store.key());
    }
    if let Err(err) = stores::refresh_library(store).await {
        eprintln!("Could not fetch the {} library: {err:?}", store.key());
    }
}

/// Opens the store's sign-in page in its own window. When the window reaches the page the
/// store lands on after signing in, the navigation is stopped, the cookies are collected,
/// and the login is completed. The result arrives as a `store-login` event.
#[tauri::command]
pub async fn start_store_login(app: AppHandle, store: String) -> Result<(), String> {
    let store = parse_store(&store)?;
    let pending = stores::pending_login(store);
    if pending.done_url_prefixes.is_empty() {
        return Err(format!(
            "{} signs in with a pasted value, not a login window",
            store.display_name()
        ));
    }
    let label = format!("login-{}", store.key());
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return Ok(());
    }
    let start_url = pending
        .start_url
        .parse()
        .map_err(|err| format!("Bad sign-in URL: {err}"))?;

    let finished = Arc::new(AtomicBool::new(false));
    let pending = Arc::new(pending);
    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(start_url))
        .title(format!("Sign in to {}", store.display_name()))
        .inner_size(520.0, 760.0)
        // A private session per login: nothing of a previous sign-in leaks in or stays behind.
        .incognito(true);
    if let Some(user_agent) = pending.user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }
    let on_navigation = {
        let app = app.clone();
        let label = label.clone();
        let finished = finished.clone();
        let pending = pending.clone();
        move |url: &tauri::Url| {
            let reached_done = pending
                .done_url_prefixes
                .iter()
                .any(|prefix| url.as_str().starts_with(prefix.as_str()));
            if !reached_done {
                return true;
            }
            if !finished.swap(true, Ordering::SeqCst) {
                tauri::async_runtime::spawn(finish_login(
                    app.clone(),
                    label.clone(),
                    pending.clone(),
                    url.to_string(),
                ));
            }
            false
        }
    };
    let window = builder
        .on_navigation(on_navigation)
        .build()
        .map_err(|err| format!("Could not open the sign-in window: {err}"))?;

    let app_for_close = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) && !finished.swap(true, Ordering::SeqCst)
        {
            let _ = app_for_close.emit(
                "store-login",
                LoginEvent {
                    store: store.key().to_string(),
                    ok: false,
                    message: "Sign-in window closed".to_string(),
                },
            );
        }
    });
    Ok(())
}

async fn finish_login(
    app: AppHandle,
    label: String,
    pending: Arc<stores::PendingLogin>,
    final_url: String,
) {
    let store = pending.store;
    let cookies = collect_cookies(&app, &label, &pending.cookie_urls).await;
    let capture = stores::LoginCapture {
        final_url: Some(final_url),
        cookies,
        pasted: None,
    };
    let result = stores::complete_login(&pending, &capture).await;
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.destroy();
    }
    let event = match result {
        Ok(name) => {
            after_sign_in(store).await;
            LoginEvent {
                store: store.key().to_string(),
                ok: true,
                message: match name {
                    Some(name) => format!("Signed in as {name}"),
                    None => "Signed in".to_string(),
                },
            }
        }
        Err(err) => LoginEvent {
            store: store.key().to_string(),
            ok: false,
            message: err.to_string(),
        },
    };
    let _ = app.emit("store-login", event);
}

/// Reads the login window's cookies. WebView2 deadlocks when cookies are read from an event
/// handler, so this runs on a blocking thread.
async fn collect_cookies(app: &AppHandle, label: &str, urls: &[String]) -> Vec<stores::Cookie> {
    let Some(window) = app.get_webview_window(label) else {
        return vec![];
    };
    let urls = urls.to_vec();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut cookies = vec![];
        for url in urls {
            let Ok(parsed) = url.parse::<tauri::Url>() else {
                continue;
            };
            let host = parsed.host_str().unwrap_or_default().to_string();
            match window.cookies_for_url(parsed) {
                Ok(found) => cookies.extend(found.into_iter().map(|cookie| {
                    stores::Cookie {
                        name: cookie.name().to_string(),
                        value: cookie.value().to_string(),
                        domain: cookie
                            .domain()
                            .map(str::to_string)
                            .unwrap_or_else(|| host.clone()),
                    }
                })),
                Err(err) => eprintln!("Could not read cookies for {url}: {err}"),
            }
        }
        cookies
    })
    .await;
    let mut cookies = result.unwrap_or_default();
    cookies.sort_by(|a, b| (&a.domain, &a.name).cmp(&(&b.domain, &b.name)));
    cookies.dedup_by(|a, b| a.domain == b.domain && a.name == b.name);
    cookies
}
