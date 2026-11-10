//! Shared HTTP helpers for the store clients.

use reqwest::{Client, RequestBuilder, Response};
use serde::de::DeserializeOwned;

use crate::Cookie;

/// A browser-like user agent. Several store web endpoints reject unknown clients; store
/// modules override it per request where a launcher's own user agent is expected.
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

pub fn client() -> eyre::Result<Client> {
    Ok(Client::builder()
        .user_agent(BROWSER_USER_AGENT)
        .cookie_store(true)
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

/// A `Cookie` header value for the cookies whose domain matches `host` (a cookie domain of
/// `.example.com` matches `example.com` and every subdomain).
pub fn cookie_header(cookies: &[Cookie], host: &str) -> String {
    cookies
        .iter()
        .filter(|c| domain_matches(&c.domain, host))
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// The store refused the saved login: HTTP 401 or 403, or a sign-in page where data was
/// expected. Store modules check for it with [`is_login_rejected`] to renew the token once
/// and try again.
#[derive(Debug)]
pub struct LoginRejected(pub String);

impl std::fmt::Display for LoginRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoginRejected {}

pub fn login_rejected(message: impl Into<String>) -> eyre::Report {
    eyre::Report::new(LoginRejected(message.into()))
}

pub fn is_login_rejected(err: &eyre::Report) -> bool {
    err.chain().any(|cause| cause.is::<LoginRejected>())
}

/// Sends a request and decodes a JSON body, turning HTTP errors into readable messages that
/// include the start of the response body.
pub async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> eyre::Result<T> {
    let response = send(request).await?;
    let url = without_query(response.url());
    let text = response.text().await.map_err(request_failed)?;
    serde_json::from_str(&text).map_err(|err| {
        eyre::format_err!(
            "Unexpected response from {url}: {err}. Body starts with: {}",
            snippet(&text)
        )
    })
}

/// Sends a request and fails on a non-success status.
pub async fn send(request: RequestBuilder) -> eyre::Result<Response> {
    let response = request.send().await.map_err(request_failed)?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let url = without_query(response.url());
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(login_rejected(format!(
            "The store rejected the saved login ({status} from {url}). Sign in again. {}",
            snippet(&body)
        )));
    }
    Err(eyre::format_err!("{status} from {url}: {}", snippet(&body)))
}

/// A reqwest error as a message without the request's query string, which can hold tokens.
pub fn request_failed(err: reqwest::Error) -> eyre::Report {
    let at = err.url().map(without_query);
    let err = err.without_url();
    let mut message = err.to_string();
    let mut cause = std::error::Error::source(&err);
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    match at {
        Some(url) => eyre::format_err!("{message} ({url})"),
        None => eyre::format_err!("{message}"),
    }
}

/// The URL without query, fragment and user info: what is safe to show in an error.
pub fn without_query(url: &reqwest::Url) -> String {
    let mut url = url.clone();
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_password(None);
    let _ = url.set_username("");
    url.to_string()
}

/// The start of a response body for an error message, with secrets taken out.
pub fn snippet(text: &str) -> String {
    redact(text).chars().take(300).collect()
}

/// Removes what may be a secret from text that ends up in errors and logs: the query string
/// and fragment of every URL (tokens, client secrets, Humble order keys), and the values of
/// JSON fields whose names suggest a credential.
pub fn redact(text: &str) -> String {
    redact_json_secrets(&redact_urls(text))
}

fn redact_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| rest.find(scheme))
        .min()
    {
        let (before, from_url) = rest.split_at(start);
        out.push_str(before);
        let end = from_url
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '}' | '\\')
            })
            .unwrap_or(from_url.len());
        let (url, after) = from_url.split_at(end);
        let kept = url.find(['?', '#']).unwrap_or(url.len());
        out.push_str(url.get(..kept).unwrap_or(url));
        rest = after;
    }
    out.push_str(rest);
    out
}

const SECRET_FIELD_PARTS: &[&str] = &[
    "token", "secret", "password", "cookie", "code", "key", "session", "auth", "ticket", "sid",
];

fn is_secret_field(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !name.starts_with("error") && SECRET_FIELD_PARTS.iter().any(|part| name.contains(part))
}

/// Replaces the string values of secret-looking JSON fields with `"…"`. Works on cut-off or
/// otherwise broken JSON too.
fn redact_json_secrets(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c != '"' {
            out.push(c);
            i += 1;
            continue;
        }
        let name_end = string_end(&chars, i);
        let name: String = chars
            .get(i + 1..name_end.saturating_sub(1).max(i + 1))
            .unwrap_or_default()
            .iter()
            .collect();
        out.extend(chars.get(i..name_end).unwrap_or_default());
        i = name_end;
        let colon = skip_whitespace(&chars, i);
        if chars.get(colon) != Some(&':') || !is_secret_field(&name) {
            continue;
        }
        let value = skip_whitespace(&chars, colon + 1);
        if chars.get(value) != Some(&'"') {
            continue;
        }
        out.extend(chars.get(i..value).unwrap_or_default());
        out.push_str("\"…\"");
        i = string_end(&chars, value);
    }
    out
}

/// The index after the closing quote of the string that opens at `open`.
fn string_end(chars: &[char], open: usize) -> usize {
    let mut i = open + 1;
    while let Some(&c) = chars.get(i) {
        match c {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

fn skip_whitespace(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

/// Reads a query parameter from a URL.
pub fn query_param(url: &str, name: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// A client whose cookie jar is seeded from a saved session, so cookies follow redirects and
/// cookies a store renews can be read back into the session. Shared with the other
/// cookie-session stores (Battle.net).
pub mod cookie_jar {
    use std::sync::Arc;

    use reqwest::{cookie::Jar, Client};

    use crate::{http, Cookie};

    pub(crate) fn client_with(cookies: &[Cookie]) -> eyre::Result<(Client, Arc<Jar>)> {
        let jar = Arc::new(Jar::default());
        for cookie in cookies {
            add(&jar, cookie);
        }
        let client = Client::builder()
            .user_agent(http::BROWSER_USER_AGENT)
            .cookie_provider(jar.clone())
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok((client, jar))
    }

    /// Webviews report domain cookies with a leading dot (`.ea.com`) and host-only cookies
    /// without (`accounts.ea.com`). Host-only cookies stay host-only, so a cookie the store
    /// renews replaces the saved one instead of being sent next to it.
    fn add(jar: &Jar, cookie: &Cookie) {
        let host = cookie.domain.trim_start_matches('.');
        if host.is_empty() || cookie.name.is_empty() {
            return;
        }
        let Ok(url) = url::Url::parse(&format!("https://{host}/")) else {
            return;
        };
        let attributes = if cookie.domain.starts_with('.') {
            format!("Domain={host}; Path=/")
        } else {
            "Path=/".to_string()
        };
        jar.add_cookie_str(
            &format!("{}={}; {attributes}", cookie.name, cookie.value),
            &url,
        );
    }

    /// The `name=value` pairs the jar would send to `host`.
    pub(crate) fn pairs_for(jar: &Jar, host: &str) -> Vec<(String, String)> {
        use reqwest::cookie::CookieStore;
        let Ok(url) = url::Url::parse(&format!("https://{host}/")) else {
            return vec![];
        };
        let Some(header) = jar.cookies(&url) else {
            return vec![];
        };
        header
            .to_str()
            .unwrap_or_default()
            .split(';')
            .filter_map(|pair| {
                let (name, value) = pair.trim().split_once('=')?;
                Some((name.to_string(), value.to_string()))
            })
            .collect()
    }

    /// Copies renewed cookie values from the jar into `cookies`, and adds cookies the stores
    /// set for `hosts` since.
    pub(crate) fn refresh(cookies: &mut Vec<Cookie>, jar: &Jar, hosts: &[&str]) {
        for host in hosts {
            for (name, value) in pairs_for(jar, host) {
                match cookies
                    .iter_mut()
                    .find(|c| c.name == name && http::domain_matches(&c.domain, host))
                {
                    Some(existing) => existing.value = value,
                    None => cookies.push(Cookie {
                        name,
                        value,
                        domain: host.to_string(),
                    }),
                }
            }
        }
    }

    /// The cookies of `domain` and its subdomains, one per name and domain (the last wins).
    pub(crate) fn keep_domain(cookies: &[Cookie], domain: &str) -> Vec<Cookie> {
        let mut kept: Vec<Cookie> = Vec::new();
        for cookie in cookies {
            let host = cookie.domain.trim_start_matches('.');
            if host.is_empty() || !http::domain_matches(domain, host) {
                continue;
            }
            kept.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain));
            kept.push(cookie.clone());
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_header_keeps_matching_domains() {
        let cookies = vec![
            Cookie {
                name: "a".into(),
                value: "1".into(),
                domain: ".humblebundle.com".into(),
            },
            Cookie {
                name: "b".into(),
                value: "2".into(),
                domain: "other.com".into(),
            },
        ];
        assert_eq!(cookie_header(&cookies, "www.humblebundle.com"), "a=1");
        assert_eq!(cookie_header(&cookies, "humblebundle.com"), "a=1");
        assert_eq!(cookie_header(&cookies, "nothumblebundle.com"), "");
    }

    #[test]
    fn redacts_secrets_in_urls_and_json() {
        let message = r#"400 Bad Request from https://auth.gog.com/token?client_id=1&refresh_token=SECRET: {"error":"invalid_grant","refresh_token": "SECRET2","access_token":"SEC\"RET3","count":2}"#;
        let redacted = redact(message);
        assert!(!redacted.contains("SECRET"), "{redacted}");
        assert!(!redacted.contains("RET3"), "{redacted}");
        assert!(
            redacted.contains("https://auth.gog.com/token "),
            "{redacted}"
        );
        assert!(
            redacted.contains(r#""error":"invalid_grant""#),
            "{redacted}"
        );
        assert!(redacted.contains(r#""count":2"#), "{redacted}");
        // Cut-off JSON, as in a snippet.
        assert_eq!(redact(r#"{"gamekey":"abc"#), r#"{"gamekey":"…""#);
        assert_eq!(
            redact("see (https://x.com/a?k=1#f) ok"),
            "see (https://x.com/a) ok"
        );
        let url =
            reqwest::Url::parse("https://u:p@humblebundle.com/api/v1/order/K?all_tpkds=true#x")
                .map(|u| without_query(&u))
                .unwrap_or_default();
        assert_eq!(url, "https://humblebundle.com/api/v1/order/K");
    }

    #[test]
    fn reads_query_parameters() {
        assert_eq!(
            query_param(
                "https://embed.gog.com/on_login_success?origin=client&code=XYZ",
                "code"
            ),
            Some("XYZ".to_string())
        );
    }
}
