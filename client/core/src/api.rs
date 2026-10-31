//! The OpenVAIR key holder API (`/api/key/*`): everything the desktop client
//! needs is read with the key itself, so it never holds an account session.
use crate::{CoreError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub const DEFAULT_SERVER: &str = "http://localhost:8080";

/// Accepts `openvair.example.com` or a full URL and keeps only the origin.
/// Plain HTTP is accepted anywhere; [`is_public_plain_http`] tells callers
/// when keys would cross the internet unencrypted.
pub fn normalize_server(input: &str) -> Result<Url> {
    let trimmed = input.trim().trim_end_matches('/');
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| CoreError::Other("服务器地址无效".into()))?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err(CoreError::Other(
            "服务器地址必须以 https:// 或 http:// 开头".into(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    Url::parse(&origin).map_err(|_| CoreError::Other("服务器地址无效".into()))
}

/// Self-hosted servers often sit on a LAN or a tailnet without a certificate:
/// this machine, private and CGNAT (Tailscale) addresses, and names that only
/// resolve inside a network.
fn is_private_network(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            !host.contains('.')
                || [".localhost", ".local", ".lan", ".internal", ".home.arpa"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
        }
        Some(url::Host::Ipv4(ip)) => {
            let [a, b, ..] = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || (a == 100 && (64..128).contains(&b))
        }
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
        None => false,
    }
}

/// Whether traffic to the server, keys included, travels unencrypted.
pub fn is_plain_http(server: &Url) -> bool {
    server.scheme() == "http"
}

/// Plain HTTP to an address outside this machine and its networks: anyone on
/// the way can read the keys, so the user has to agree to it first.
pub fn is_public_plain_http(server: &Url) -> bool {
    is_plain_http(server) && !is_private_network(server)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub enabled: bool,
    pub can_copy: bool,
    pub used_cost_today_usd: f64,
    pub cost_limit_daily_usd: Option<f64>,
    pub models: BTreeMap<String, BTreeSet<String>>,
}

/// A key as its holder sees it (`/api/key/summary`): no account needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySummary {
    pub id: String,
    pub account_name: String,
    pub key_name: String,
    pub key_prefix: String,
    /// `active`, `paused`, `exhausted_daily`, `exhausted_total` or `cost_paused`.
    pub status: String,
    pub requests_per_minute: Option<i64>,
    pub concurrency_limit: i64,
    pub token_limit_daily: Option<i64>,
    pub cost_limit_daily_usd: Option<f64>,
    pub cost_limit_total_usd: Option<f64>,
    pub used_tokens_today: i64,
    pub used_cost_today_usd: f64,
    pub used_cost_total_usd: f64,
    pub day_ends_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub models: BTreeMap<String, BTreeSet<String>>,
    /// The address the server's administrator set for it, when there is one.
    #[serde(default)]
    pub server_url: Option<String>,
}

impl KeySummary {
    /// The key in the shape the rest of the app lists keys in.
    pub fn as_client_key(&self) -> ClientKey {
        ClientKey {
            id: self.id.clone(),
            name: self.key_name.clone(),
            key_prefix: self.key_prefix.clone(),
            enabled: self.status != "paused",
            can_copy: true,
            used_cost_today_usd: self.used_cost_today_usd,
            cost_limit_daily_usd: self.cost_limit_daily_usd,
            models: self.models.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Api {
    pub server: Url,
    http: reqwest::Client,
}

impl Api {
    pub fn new(server: Url) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("OpenVAIR-Desktop/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("HTTP client configuration is static");
        Self { server, http }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.server.as_str().trim_end_matches('/'), path)
    }

    async fn read<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CoreError::Unauthorized);
        }
        if !status.is_success() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            let message = body["error"]["message"]
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("服务器返回 {}", status.as_u16()));
            return Err(CoreError::Api {
                status: status.as_u16(),
                message,
            });
        }
        Ok(response.json().await?)
    }

    /// Checks a key and reads its status. The key is the credential here, so
    /// a rejected key surfaces as `Unauthorized`.
    pub async fn key_summary(&self, secret: &str) -> Result<KeySummary> {
        let secret = secret.trim();
        if secret.is_empty() {
            return Err(CoreError::Other("请粘贴 OpenVAIR Key".into()));
        }
        let response = self
            .http
            .get(self.url("/api/key/summary"))
            .bearer_auth(secret)
            .send()
            .await?;
        Self::read(response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_normalize_to_an_https_origin() {
        assert_eq!(
            normalize_server("openvair.example.com/").unwrap().as_str(),
            "https://openvair.example.com/"
        );
        assert_eq!(
            normalize_server("https://test.openvair.example.com/keys?x=1")
                .unwrap()
                .as_str(),
            "https://test.openvair.example.com/"
        );
        assert!(normalize_server("ftp://openvair.example.com").is_err());
        assert!(!is_plain_http(&normalize_server("10.0.0.2").unwrap()));
        assert!(is_plain_http(&normalize_server("http://10.0.0.2").unwrap()));
    }

    #[test]
    fn plain_http_is_allowed_but_flagged_outside_private_networks() {
        let public = |input: &str| is_public_plain_http(&normalize_server(input).unwrap());
        for private in [
            "http://localhost:8080",
            "http://192.168.1.20:8080",
            "http://[fd00::1]:8080",
            "http://100.101.2.3:8080",
            "http://vair:8080",
            "http://vair.lan",
            "http://nas.home.arpa",
        ] {
            assert!(!public(private), "{private}");
        }
        for internet in [
            "http://openvair.example.com",
            "http://8.8.8.8:8080",
            "http://100.128.0.1",
        ] {
            assert!(public(internet), "{internet}");
        }
        assert!(!public("openvair.example.com"));
    }

    #[test]
    fn key_summaries_read_the_server_shape_and_list_as_one_key() {
        let body = serde_json::json!({
            "id": "6f1c0e1a-0000-4000-8000-000000000001", "account_name": "Lab",
            "key_name": "student", "key_prefix": "openvair_Ab3x", "status": "paused",
            "allowed_models": [], "model_mode": "all", "requests_per_minute": null,
            "concurrency_limit": 4, "token_limit_daily": 1000, "cost_limit_daily_usd": null,
            "cost_limit_total_usd": 50.0, "used_tokens_today": 10, "used_cost_today_usd": 0.5,
            "used_cost_total_usd": 3.25, "day_started_at": "2026-10-12T00:00:00+00:00",
            "day_ends_at": "2026-10-13T00:00:00+00:00", "created_at": "2026-10-01T00:00:00Z",
            "last_used_at": null, "models": {"openai": ["gpt-a"]}
        });
        let summary: KeySummary = serde_json::from_value(body).unwrap();
        let key = summary.as_client_key();
        assert_eq!(key.name, "student");
        assert!(!key.enabled, "a paused key cannot be written to agents");
        assert!(key.can_copy);
        assert_eq!(key.models["openai"].len(), 1);
    }
}
