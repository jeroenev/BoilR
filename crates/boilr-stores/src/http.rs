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

/// Sends a request and decodes a JSON body, turning HTTP errors into readable messages that
/// include the start of the response body.
pub async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> eyre::Result<T> {
    let response = send(request).await?;
    let url = response.url().to_string();
    let text = response.text().await?;
    serde_json::from_str(&text).map_err(|err| {
        eyre::format_err!(
            "Unexpected response from {url}: {err}. Body starts with: {}",
            snippet(&text)
        )
    })
}

/// Sends a request and fails on a non-success status.
pub async fn send(request: RequestBuilder) -> eyre::Result<Response> {
    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let url = response.url().to_string();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(eyre::format_err!(
            "The store rejected the saved login ({status} from {url}). Sign in again. {}",
            snippet(&body)
        ));
    }
    Err(eyre::format_err!("{status} from {url}: {}", snippet(&body)))
}

pub fn snippet(text: &str) -> String {
    text.chars().take(300).collect()
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
