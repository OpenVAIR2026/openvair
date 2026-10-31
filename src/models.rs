//! Catalog membership describes names; it never grants request authorization.
use crate::{auth, error::Result, model_rules, state::AppState};
use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, FromRow, Serialize)]
pub struct CatalogName {
    pub model: String,
    pub protocol: String,
}

#[derive(FromRow)]
pub struct ChannelScope {
    pub id: Uuid,
    pub protocol: String,
    pub models: Value,
    pub mapping: Value,
    pub model_mode: String,
    pub routable: bool,
    pub excluded_models: Vec<String>,
}

impl ChannelScope {
    pub fn accepts(&self, model: &str) -> bool {
        model_rules::concrete(model)
            && (self.model_mode == "passthrough"
                || self.mapping.get(model).is_some()
                || self.models.as_array().is_some_and(|rules| {
                    rules
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|rule| model_rules::matches(rule, model))
                }))
    }
}

pub async fn directory(db: &PgPool) -> Result<Vec<CatalogName>> {
    Ok(sqlx::query_as(
        "SELECT pattern AS model, unnest(protocols) AS protocol FROM model_catalog
         WHERE enabled AND strpos(pattern,'*')=0
         ORDER BY model,protocol",
    )
    .fetch_all(db)
    .await?)
}

pub async fn scopes(db: &PgPool, account_id: Uuid) -> Result<Vec<ChannelScope>> {
    Ok(sqlx::query_as(
        "SELECT c.id,c.protocol,c.models,c.model_mode,
                COALESCE(r.mapping,'{}'::jsonb) || c.model_mapping AS mapping,
                -- Unavailable channels stay routable as a last resort.
                (c.enabled AND EXISTS(SELECT 1 FROM channel_keys k WHERE k.channel_id=c.id AND k.enabled)) AS routable,
                ARRAY(SELECT qm.model FROM quality_monitors qm WHERE qm.channel_id=c.id
                      AND qm.deleted_at IS NULL AND qm.route_policy='exclude'
                      AND ((qm.kind='service' AND qm.status='degraded' AND qm.valid_until>now())
                        OR (qm.kind='authenticity' AND qm.window_verdict='confirmed' AND qm.window_valid_until>now()))) AS excluded_models
         FROM channels c
         LEFT JOIN LATERAL (
            SELECT jsonb_object_agg(public_model,upstream_model) AS mapping FROM (
              SELECT DISTINCT ON(public_model) public_model,upstream_model FROM model_routes
              WHERE channel_id=c.id AND account_id=$1 AND enabled AND protocol=c.protocol
              ORDER BY public_model,priority,id
            ) selected
         ) r ON true WHERE c.account_id=$1 ORDER BY c.priority,c.created_at,c.id"
    ).bind(account_id).fetch_all(db).await?)
}

/// Concrete names a channel serves: catalog names its rules accept, plus the
/// concrete names, aliases and routes the channel spells out, so configured models
/// missing from the catalog are still listed. Fetched upstream models stay
/// form suggestions only.
pub fn channel_names(directory: &[CatalogName], channel: &ChannelScope) -> Vec<String> {
    let own = channel
        .models
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .chain(
            channel
                .mapping
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, _)| name.clone()),
        );
    directory
        .iter()
        .filter(|name| name.protocol == channel.protocol)
        .map(|name| name.model.clone())
        .chain(own)
        .filter(|name| channel.accepts(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub async fn routable_catalog(db: &PgPool, account_id: Uuid) -> Result<BTreeMap<String, String>> {
    routable_catalog_for(db, account_id, None).await
}

/// `channels` restricts the catalog to a key's selected channels.
pub async fn routable_catalog_for(
    db: &PgPool,
    account_id: Uuid,
    channels: Option<&[Uuid]>,
) -> Result<BTreeMap<String, String>> {
    let directory = directory(db).await?;
    let mut result = BTreeMap::new();
    for channel in scopes(db, account_id).await?.iter().filter(|channel| {
        channel.routable && channels.is_none_or(|selected| selected.contains(&channel.id))
    }) {
        for name in channel_names(&directory, channel) {
            if channel.excluded_models.contains(&name) {
                continue;
            }
            result
                .entry(name)
                .or_insert_with(|| channel.protocol.clone());
        }
    }
    Ok(result)
}

/// Routable public models grouped by protocol. Unlike `routable_catalog`, a
/// model served on several protocols is listed under each of them, because a
/// client picks models for one protocol at a time.
pub async fn routable_by_protocol(
    db: &PgPool,
    account_id: Uuid,
    channels: Option<&[Uuid]>,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let directory = directory(db).await?;
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for channel in scopes(db, account_id).await?.iter().filter(|channel| {
        channel.routable && channels.is_none_or(|selected| selected.contains(&channel.id))
    }) {
        let names = channel_names(&directory, channel)
            .into_iter()
            .filter(|name| !channel.excluded_models.contains(name));
        result
            .entry(channel.protocol.clone())
            .or_default()
            .extend(names);
    }
    Ok(result)
}

pub async fn available(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BTreeMap<String, String>>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(routable_catalog(&state.db, user.account_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope(mode: &str, models: Value, mapping: Value) -> ChannelScope {
        ChannelScope {
            id: Uuid::nil(),
            protocol: "anthropic".into(),
            models,
            mapping,
            model_mode: mode.into(),
            routable: true,
            excluded_models: Vec::new(),
        }
    }

    #[test]
    fn channel_names_include_models_missing_from_the_catalog() {
        let directory = vec![
            CatalogName {
                model: "kimi-k2".into(),
                protocol: "openai".into(),
            },
            CatalogName {
                model: "claude-x".into(),
                protocol: "anthropic".into(),
            },
        ];
        let channel = scope(
            "allowlist",
            json!(["kimi-k3", "glm-*"]),
            json!({"fast": "kimi-k3-turbo"}),
        );
        assert_eq!(channel_names(&directory, &channel), ["fast", "kimi-k3"]);
        let passthrough = scope("passthrough", json!([]), json!({}));
        assert_eq!(channel_names(&directory, &passthrough), ["claude-x"]);
    }
}
