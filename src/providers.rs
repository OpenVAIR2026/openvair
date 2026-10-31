//! Vendor presets that fill a new channel's protocol and base URL. Presets only
//! supply defaults: routing, auth and metering still follow the channel's
//! protocol, and the user may edit the base URL. Presets list mainland
//! platforms only; keys from international platforms need a custom channel.

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{auth, error::Result, state::AppState};

#[derive(Serialize)]
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub docs_url: &'static str,
    pub endpoints: &'static [Endpoint],
}

#[derive(Serialize)]
pub struct Endpoint {
    pub protocol: &'static str,
    pub base_url: &'static str,
}

const fn endpoint(protocol: &'static str, base_url: &'static str) -> Endpoint {
    Endpoint { protocol, base_url }
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "deepseek",
        label: "DeepSeek",
        docs_url: "https://api-docs.deepseek.com/",
        endpoints: &[
            endpoint("openai", "https://api.deepseek.com"),
            endpoint("anthropic", "https://api.deepseek.com/anthropic"),
        ],
    },
    Provider {
        id: "moonshot",
        label: "Kimi（月之暗面）",
        docs_url: "https://platform.kimi.com/docs/api/overview",
        endpoints: &[
            endpoint("openai", "https://api.moonshot.cn/v1"),
            endpoint("anthropic", "https://api.moonshot.cn/anthropic"),
        ],
    },
    Provider {
        id: "zhipu",
        label: "智谱 GLM",
        docs_url: "https://docs.bigmodel.cn/",
        endpoints: &[
            endpoint("openai", "https://open.bigmodel.cn/api/paas/v4"),
            endpoint("anthropic", "https://open.bigmodel.cn/api/anthropic"),
        ],
    },
    Provider {
        id: "dashscope",
        label: "通义千问（阿里云百炼）",
        docs_url: "https://help.aliyun.com/zh/model-studio/",
        endpoints: &[
            endpoint(
                "openai",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ),
            endpoint("anthropic", "https://dashscope.aliyuncs.com/apps/anthropic"),
        ],
    },
    Provider {
        id: "minimax",
        label: "MiniMax",
        docs_url: "https://platform.minimaxi.com/",
        endpoints: &[
            endpoint("openai", "https://api.minimaxi.com/v1"),
            endpoint("anthropic", "https://api.minimaxi.com/anthropic"),
        ],
    },
    Provider {
        id: "volcengine",
        label: "豆包（火山方舟）",
        docs_url: "https://www.volcengine.com/docs/82379",
        endpoints: &[
            endpoint("openai", "https://ark.cn-beijing.volces.com/api/v3"),
            endpoint(
                "anthropic",
                "https://ark.cn-beijing.volces.com/api/compatible",
            ),
        ],
    },
];

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// Whether `id` names a preset with an endpoint for `protocol`.
pub fn supports(id: &str, protocol: &str) -> bool {
    find(id).is_some_and(|provider| {
        provider
            .endpoints
            .iter()
            .any(|endpoint| endpoint.protocol == protocol)
    })
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<&'static [Provider]>> {
    auth::current_user(&state, &headers).await?;
    Ok(Json(PROVIDERS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::join_base_url;

    #[test]
    fn presets_are_unique_and_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for provider in PROVIDERS {
            assert!(ids.insert(provider.id), "duplicate {}", provider.id);
            assert!(provider.docs_url.starts_with("https://"));
            let mut seen = std::collections::HashSet::new();
            for endpoint in provider.endpoints {
                assert!(matches!(endpoint.protocol, "openai" | "anthropic"));
                assert!(endpoint.base_url.starts_with("https://"));
                assert!(!endpoint.base_url.ends_with('/'));
                assert!(
                    seen.insert(endpoint.protocol),
                    "{} repeats an endpoint",
                    provider.id
                );
            }
        }
    }

    #[test]
    fn preset_bases_join_to_vendor_paths() {
        for provider in PROVIDERS {
            for endpoint in provider.endpoints {
                let path = if endpoint.protocol == "anthropic" {
                    "/v1/messages"
                } else {
                    "/v1/chat/completions"
                };
                let target = join_base_url(endpoint.base_url, path).unwrap();
                let suffix = if endpoint.protocol == "anthropic" {
                    "/v1/messages"
                } else {
                    "/chat/completions"
                };
                assert!(target.ends_with(suffix), "{target}");
                assert!(!target.contains("/v1/v1/"), "{target}");
                assert!(!target.contains("/v4/v1/"), "{target}");
                assert!(!target.contains("/v3/v1/"), "{target}");
            }
        }
        assert_eq!(
            join_base_url("https://api.deepseek.com", "/v1/chat/completions").unwrap(),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn supports_checks_protocol() {
        assert!(supports("deepseek", "anthropic"));
        assert!(!supports("deepseek", "gemini"));
        assert!(!supports("unknown", "openai"));
    }
}
