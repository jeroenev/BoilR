//! Placeholder: replaced by the store client.

use reqwest::Client;

use crate::{LoginCapture, ManualLogin, OwnedGame, PendingLogin, Session, StoreId};

pub fn begin_login() -> PendingLogin {
    PendingLogin {
        store: StoreId::Ea,
        start_url: String::new(),
        done_url_prefixes: vec![],
        cookie_urls: vec![],
        manual: ManualLogin::Unsupported,
        state: serde_json::Value::Null,
    }
}

pub async fn complete_login(
    _client: &Client,
    _pending: &PendingLogin,
    _capture: &LoginCapture,
) -> eyre::Result<Session> {
    Err(eyre::format_err!("Not implemented yet"))
}

pub async fn fetch_owned(_client: &Client, _session: &mut Session) -> eyre::Result<Vec<OwnedGame>> {
    Err(eyre::format_err!("Not implemented yet"))
}
