//! The address colleagues reach this server at, set by an administrator.
//! Without it the console shows whatever address the browser opened, which is
//! `localhost` when the administrator works on the server itself.
use crate::{
    AppState, audit, auth,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PublicUrlSetting {
    url: Option<String>,
}

/// The configured origin, without a trailing slash; `None` when unset.
pub async fn load(db: &PgPool) -> Option<String> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value_ciphertext FROM system_settings WHERE key='public_url'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    row.and_then(|json| serde_json::from_str::<PublicUrlSetting>(&json).ok())
        .and_then(|setting| setting.url)
}

/// Accepts `https://host[:port]` or `http://…`, adding `https://` when the
/// scheme is missing. Paths are refused because the console and the gateway
/// are served from the root.
fn normalize(input: &str) -> Result<Option<String>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let invalid = || AppError::BadRequest("访问地址无效，请填写例如 https://ai.example.com".into());
    let url = Url::parse(&with_scheme).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err(AppError::BadRequest(
            "访问地址必须以 https:// 或 http:// 开头".into(),
        ));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::BadRequest(
            "访问地址只填协议和域名（或 IP、端口），不要带路径".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid());
    }
    Ok(Some(url.origin().ascii_serialization()))
}

/// GET /api/server — public, so the sign-in page, the key page and the docs
/// can show the address too.
pub async fn server_info(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "public_url": load(&state.db).await }))
}

pub async fn get_setting(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(json!({ "url": load(&state.db).await })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    url: Option<String>,
}

pub async fn update_setting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateInput>,
) -> Result<Json<Value>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let url = match input.url.as_deref() {
        Some(url) => normalize(url)?,
        None => None,
    };
    let value = serde_json::to_string(&PublicUrlSetting { url: url.clone() })
        .map_err(|_| AppError::Internal("public url serialization failed".into()))?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ('public_url',$1)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(&value)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.public_url_updated",
        "system_settings",
        user.user_id,
        json!({ "url": &url }),
    )
    .await;
    Ok(Json(json!({ "url": url })))
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn addresses_are_reduced_to_their_origin() {
        let ok = |input: &str| normalize(input).unwrap();
        assert_eq!(ok(""), None);
        assert_eq!(ok("  "), None);
        assert_eq!(ok("ai.example.com"), Some("https://ai.example.com".into()));
        assert_eq!(
            ok("https://AI.example.com/"),
            Some("https://ai.example.com".into())
        );
        assert_eq!(
            ok("http://192.168.1.20:8080"),
            Some("http://192.168.1.20:8080".into())
        );
        assert_eq!(
            ok("https://ai.example.com:443"),
            Some("https://ai.example.com".into())
        );
        for bad in [
            "ftp://ai.example.com",
            "https://ai.example.com/vair",
            "https://u:p@ai.example.com",
            "https://ai.example.com?x=1",
            "http://",
        ] {
            assert!(normalize(bad).is_err(), "{bad}");
        }
    }
}
