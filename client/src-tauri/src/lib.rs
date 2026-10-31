//! Tauri commands: a thin layer over `openvair-core` for the React UI. Secrets
//! stay on this side; the UI only ever sees masked values.
//!
//! OpenVAIR Desktop runs on keys alone: the user adds one or more keys from
//! one OpenVAIR server and chooses one for each agent.
mod tray;

use openvair_core::{
    CoreError, Result,
    agents::{self, AgentId, Change, Protocol, Settings},
    api::{self, Api, ClientKey, KeySummary},
    bili,
    credentials::Credentials,
    probe::{self, TestResult},
    state::{self, Connection, LocalState, SavedKey},
};
use serde::Serialize;
use std::path::PathBuf;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::Mutex;
use url::Url;

struct Inner {
    data_dir: PathBuf,
    home: PathBuf,
    local: LocalState,
}

impl Inner {
    fn saved(&self, key_id: &str) -> Result<&SavedKey> {
        self.local
            .key(key_id)
            .ok_or_else(|| CoreError::Other("这把 Key 不在本机的列表里".into()))
    }

    /// A saved key with its secret from the credential store.
    fn secret(&self, key_id: &str) -> Result<(SavedKey, Url, String)> {
        let key = self.saved(key_id)?.clone();
        let secret = Credentials::for_key(&key.id, &self.data_dir)
            .load()
            .ok_or_else(|| {
                CoreError::Other(format!("本机找不到「{}」的密钥，请重新添加", key.name()))
            })?;
        Ok((key, self.server(), secret))
    }

    /// The server every key and agent here uses; before the first key is
    /// added, the built-in default.
    fn server(&self) -> Url {
        self.local
            .server
            .as_deref()
            .and_then(|server| api::normalize_server(server).ok())
            .unwrap_or_else(|| {
                api::normalize_server(api::DEFAULT_SERVER).expect("default server is valid")
            })
    }
}

struct App(Mutex<Inner>);

fn host_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    host.trim_end_matches(".local").to_owned()
}

fn server_host(server: &str) -> String {
    Url::parse(server)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            })
        })
        .unwrap_or_else(|| server.to_owned())
}

/// What adding a first key to this address would mean for its security.
#[derive(Serialize)]
struct ServerCheck {
    server: String,
    plain_http: bool,
    public_http: bool,
}

#[tauri::command]
fn check_server(server: String) -> Result<ServerCheck> {
    let url = api::normalize_server(&server)?;
    Ok(ServerCheck {
        plain_http: api::is_plain_http(&url),
        public_http: api::is_public_plain_http(&url),
        server: url.to_string(),
    })
}

#[derive(Serialize)]
struct SessionView {
    host: String,
    /// The OpenVAIR server, once a key has been added.
    server: Option<String>,
    server_host: Option<String>,
    /// The server speaks plain HTTP, so keys travel unencrypted.
    plain_http: bool,
    /// ... and over the internet, not just this machine's networks.
    public_http: bool,
    key_count: usize,
}

#[tauri::command]
async fn session(app: tauri::State<'_, App>) -> Result<SessionView> {
    let inner = app.0.lock().await;
    let server = inner.local.server.clone();
    Ok(SessionView {
        host: host_name(),
        server_host: server.as_deref().map(server_host),
        plain_http: server
            .as_deref()
            .and_then(|server| Url::parse(server).ok())
            .is_some_and(|url| api::is_plain_http(&url)),
        public_http: server
            .as_deref()
            .and_then(|server| Url::parse(server).ok())
            .is_some_and(|url| api::is_public_plain_http(&url)),
        server,
        key_count: inner.local.keys.len(),
    })
}

/// A saved key with what the server says about it now. A key the server
/// rejects is `invalid`; one the server cannot be asked about only has
/// `error`.
#[derive(Serialize)]
struct KeyView {
    #[serde(flatten)]
    key: ClientKey,
    key_name: String,
    is_default: bool,
    used_by: Vec<&'static str>,
    summary: Option<KeySummary>,
    invalid: bool,
    error: Option<String>,
}

fn key_view(local: &LocalState, saved: &SavedKey, result: Result<KeySummary>) -> KeyView {
    let (summary, invalid, error) = match result {
        Ok(summary) => (Some(summary), false, None),
        Err(CoreError::Unauthorized) => (None, true, Some("Key 已失效或被撤销".to_owned())),
        Err(error) => (None, false, Some(error.to_string())),
    };
    let mut key = match &summary {
        Some(summary) => summary.as_client_key(),
        None => ClientKey {
            id: saved.id.clone(),
            name: String::new(),
            key_prefix: saved.key_prefix.clone(),
            enabled: false,
            can_copy: !invalid,
            used_cost_today_usd: 0.0,
            cost_limit_daily_usd: None,
            models: Default::default(),
        },
    };
    key.name = saved.name().to_owned();
    KeyView {
        key,
        key_name: summary
            .as_ref()
            .map_or_else(|| saved.key_name.clone(), |s| s.key_name.clone()),
        is_default: local.default_key().is_some_and(|key| key.id == saved.id),
        used_by: local
            .agents_using(&saved.id)
            .into_iter()
            .map(|agent| agent.name())
            .collect(),
        summary,
        invalid,
        error,
    }
}

/// Asks the server about each key, all at once.
async fn summaries(
    server: Url,
    keys: Vec<(SavedKey, Option<String>)>,
) -> Vec<(SavedKey, Result<KeySummary>)> {
    let tasks: Vec<_> = keys
        .into_iter()
        .map(|(key, secret)| {
            let server = server.clone();
            tauri::async_runtime::spawn(async move {
                let result = match secret {
                    Some(secret) => Api::new(server).key_summary(&secret).await,
                    None => Err(CoreError::Other(
                        "本机找不到这把 Key 的密钥，请重新添加".into(),
                    )),
                };
                (key, result)
            })
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        if let Ok(result) = task.await {
            results.push(result);
        }
    }
    results
}

#[tauri::command]
async fn keys(app: tauri::State<'_, App>) -> Result<Vec<KeyView>> {
    // The window stays usable while the server is asked.
    let (server, saved) = {
        let inner = app.0.lock().await;
        let saved: Vec<_> = inner
            .local
            .keys
            .iter()
            .map(|key| {
                let secret = Credentials::for_key(&key.id, &inner.data_dir).load();
                (key.clone(), secret)
            })
            .collect();
        (inner.server(), saved)
    };
    let results = summaries(server, saved).await;
    let mut inner = app.0.lock().await;
    let mut changed = false;
    let mut views = Vec::new();
    for (key, result) in results {
        let Some(saved) = inner.local.keys.iter_mut().find(|k| k.id == key.id) else {
            continue;
        };
        // Follow renames on the server.
        if let Ok(summary) = &result
            && saved.key_name != summary.key_name
        {
            saved.key_name = summary.key_name.clone();
            changed = true;
        }
        let saved = saved.clone();
        views.push(key_view(&inner.local, &saved, result));
    }
    if changed {
        let _ = inner.local.save(&inner.data_dir);
    }
    Ok(views)
}

/// Adds a key after checking it with the server. The first key sets the
/// server and becomes the default; later keys must belong to that server.
/// A first server on plain HTTP over the internet needs `allow_public_http`,
/// which the user gives after being told keys travel unencrypted.
#[tauri::command]
async fn add_key(
    app: tauri::State<'_, App>,
    server: Option<String>,
    secret: String,
    allow_public_http: Option<bool>,
) -> Result<KeyView> {
    let current = app.0.lock().await.local.server.clone();
    let server = match current.as_deref().or(server.as_deref()) {
        Some(server) => api::normalize_server(server)?,
        None => return Err(CoreError::Other("请填写服务器地址".into())),
    };
    if current.is_none() && api::is_public_plain_http(&server) && allow_public_http != Some(true) {
        return Err(CoreError::Other(
            "公网地址使用 http:// 时 Key 会明文传输，请先确认".into(),
        ));
    }
    let secret = secret.trim().to_owned();
    let summary = match Api::new(server.clone()).key_summary(&secret).await {
        Err(CoreError::Unauthorized) => {
            return Err(CoreError::Other(if current.is_some() {
                format!(
                    "Key 无效、已撤销，或不属于 {}",
                    server_host(server.as_str())
                )
            } else {
                "Key 无效或已撤销，请确认服务器地址和 Key".into()
            }));
        }
        result => result?,
    };
    let mut inner = app.0.lock().await;
    if inner.local.server.is_some() && inner.local.server != current {
        return Err(CoreError::Other("服务器已更换，请重新添加".into()));
    }
    if let Some(existing) = inner.local.key(&summary.id) {
        return Err(CoreError::Other(format!(
            "这把 Key 已经添加过了：{}",
            existing.name()
        )));
    }
    Credentials::for_key(&summary.id, &inner.data_dir).save(&secret)?;
    let saved = SavedKey {
        id: summary.id.clone(),
        server: server.to_string(),
        label: None,
        key_name: summary.key_name.clone(),
        key_prefix: summary.key_prefix.clone(),
        added_at: chrono::Utc::now(),
    };
    inner.local.server = Some(server.to_string());
    inner.local.keys.push(saved.clone());
    if inner.local.default_key.is_none() {
        inner.local.default_key = Some(saved.id.clone());
    }
    inner.local.save(&inner.data_dir)?;
    Ok(key_view(&inner.local, &saved, Ok(summary)))
}

/// Names the key on this computer only; an empty name shows the server's.
#[tauri::command]
async fn rename_key(app: tauri::State<'_, App>, key_id: String, label: String) -> Result<()> {
    let mut inner = app.0.lock().await;
    let label = label.trim().chars().take(60).collect::<String>();
    let key = inner
        .local
        .keys
        .iter_mut()
        .find(|key| key.id == key_id)
        .ok_or_else(|| CoreError::Other("这把 Key 不在本机的列表里".into()))?;
    key.label = (!label.is_empty()).then_some(label);
    let name = key.name().to_owned();
    for connection in inner.local.connections.values_mut() {
        if connection.key_id == key_id {
            connection.key_name = name.clone();
        }
    }
    inner.local.save(&inner.data_dir)
}

#[tauri::command]
async fn set_default_key(app: tauri::State<'_, App>, key_id: String) -> Result<()> {
    let mut inner = app.0.lock().await;
    inner.saved(&key_id)?;
    inner.local.default_key = Some(key_id);
    inner.local.save(&inner.data_dir)
}

/// Forgets a key on this computer; the key itself stays valid. Agents still
/// using it must move to another key or be disconnected first.
#[tauri::command]
async fn remove_key(app: tauri::State<'_, App>, key_id: String) -> Result<()> {
    let mut inner = app.0.lock().await;
    let users = inner.local.agents_using(&key_id);
    if !users.is_empty() {
        let names: Vec<_> = users.iter().map(|agent| agent.name()).collect();
        return Err(CoreError::Other(format!(
            "{} 正在使用这把 Key，请先改用其他 Key 或断开",
            names.join("、")
        )));
    }
    inner.local.keys.retain(|key| key.id != key_id);
    if inner.local.default_key.as_deref() == Some(key_id.as_str()) {
        inner.local.default_key = inner.local.keys.first().map(|key| key.id.clone());
    }
    // Without keys the next one may come from any server.
    if inner.local.keys.is_empty() {
        inner.local.server = None;
    }
    Credentials::for_key(&key_id, &inner.data_dir).clear();
    inner.local.save(&inner.data_dir)
}

/// Forgets the server and every key saved for it, so keys from another
/// server can be added. Agents must be disconnected first, since their
/// settings point at the old server.
/// What changing the server did. `kept` is false when the keys had to go.
#[derive(Serialize)]
struct ServerChange {
    kept: bool,
    server: Option<String>,
    /// Agents whose files now point at the new address.
    updated: Vec<&'static str>,
    /// Connected agents left alone because their files no longer hold the
    /// key written here.
    skipped: Vec<&'static str>,
}

/// Moves to `server` keeping every key, when each saved key is accepted there
/// as the same key: the server only changed its address. Connected agents are
/// rewritten to the new address. Without `server`, or with `discard_keys`
/// after the keys could not be kept, forgets every key so the user can start
/// over with another server; connected agents must be disconnected first.
#[tauri::command]
async fn change_server(
    app: tauri::State<'_, App>,
    server: Option<String>,
    allow_public_http: Option<bool>,
    discard_keys: Option<bool>,
) -> Result<ServerChange> {
    if discard_keys == Some(true) || server.is_none() {
        let mut inner = app.0.lock().await;
        if !inner.local.connections.is_empty() {
            let names: Vec<_> = inner
                .local
                .connections
                .keys()
                .map(|agent| agent.name())
                .collect();
            return Err(CoreError::Other(format!(
                "请先断开 {}，再更换服务器",
                names.join("、")
            )));
        }
        for key in std::mem::take(&mut inner.local.keys) {
            Credentials::for_key(&key.id, &inner.data_dir).clear();
        }
        inner.local.default_key = None;
        inner.local.server = None;
        inner.local.save(&inner.data_dir)?;
        return Ok(ServerChange {
            kept: false,
            server: None,
            updated: Vec::new(),
            skipped: Vec::new(),
        });
    }

    let server = api::normalize_server(server.as_deref().unwrap_or_default())?;
    let (current, keys) = {
        let inner = app.0.lock().await;
        let keys = inner
            .local
            .keys
            .iter()
            .map(|key| {
                inner
                    .secret(&key.id)
                    .map(|(saved, _, secret)| (saved, secret))
            })
            .collect::<Result<Vec<_>>>()?;
        (inner.server(), keys)
    };
    if server == current {
        return Err(CoreError::Other("新地址和当前地址相同".into()));
    }
    if api::is_public_plain_http(&server) && allow_public_http != Some(true) {
        return Err(CoreError::Other(
            "公网地址使用 http:// 时 Key 会明文传输，请先确认".into(),
        ));
    }
    let host = server_host(server.as_str());
    let api = Api::new(server.clone());
    let mut summaries = Vec::new();
    for (saved, secret) in &keys {
        match api.key_summary(secret).await {
            Ok(summary) if summary.id == saved.id => summaries.push(summary),
            Ok(_) | Err(CoreError::Unauthorized) => {
                return Err(CoreError::Other(format!(
                    "「{}」在 {host} 上无效，那里不是同一台服务器，Key 无法保留",
                    saved.name()
                )));
            }
            Err(error) => {
                return Err(CoreError::Other(format!("无法连接 {host}：{error}")));
            }
        }
    }

    let mut inner = app.0.lock().await;
    if inner.server() != current {
        return Err(CoreError::Other("服务器已更换，请重新操作".into()));
    }
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    let agents: Vec<AgentId> = inner.local.connections.keys().copied().collect();
    for agent in agents {
        let connection = inner.local.connections[&agent].clone();
        let protocol = agent.protocol_or_default(connection.protocol);
        let secret = keys
            .iter()
            .find(|(saved, _)| saved.id == connection.key_id)
            .map(|(_, secret)| secret.clone());
        let holds_key = agent.read(&inner.home).is_ok_and(|current| {
            current
                .secret
                .as_deref()
                .is_some_and(|secret| secret.starts_with(&connection.key_prefix))
        });
        let (Some(secret), true) = (secret, holds_key) else {
            skipped.push(agent.name());
            continue;
        };
        let summary = summaries
            .iter()
            .find(|summary| summary.id == connection.key_id);
        let models = summary
            .map(|summary| key_models(&summary.as_client_key(), protocol))
            .unwrap_or_else(|| connection.models.clone());
        let label = serde_json::to_value(agent)
            .ok()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "agent".into());
        state::back_up(&agent.files(&inner.home), &inner.data_dir, &label)?;
        let settings = Settings {
            server: &server,
            secret: &secret,
            model: &connection.model,
            models: &models,
            background_model: connection.background_model.as_deref(),
            protocol: Some(protocol),
        };
        // The values replaced now are OpenVAIR's own, so the user's from the
        // first connect stay what disconnecting restores.
        agent.apply(&inner.home, &settings)?;
        let entry = inner
            .local
            .connections
            .get_mut(&agent)
            .expect("listed above");
        entry.server = server.to_string();
        entry.record_models(models);
        // Undo would put the old address back.
        entry.last_backups = Vec::new();
        entry.previous = None;
        updated.push(agent.name());
    }
    for key in &mut inner.local.keys {
        key.server = server.to_string();
    }
    inner.local.server = Some(server.to_string());
    inner.local.save(&inner.data_dir)?;
    Ok(ServerChange {
        kept: true,
        server: inner.local.server.clone(),
        updated,
        skipped,
    })
}

#[tauri::command]
async fn key_summary(app: tauri::State<'_, App>, key_id: String) -> Result<KeySummary> {
    let (_, server, secret) = app.0.lock().await.secret(&key_id)?;
    Api::new(server).key_summary(&secret).await
}

/// A local connection as the UI may see it: no prior values or backups.
#[derive(Serialize)]
struct ConnectionView {
    server: String,
    key_id: String,
    key_name: String,
    key_prefix: String,
    model: String,
    connected_at: chrono::DateTime<chrono::Utc>,
    last_test: Option<TestResult>,
    can_undo: bool,
    background_model: Option<String>,
    /// The background model is no longer among the key's models.
    background_missing: bool,
    models_update: Option<state::ModelsUpdate>,
}

#[derive(Serialize)]
struct AgentView {
    id: AgentId,
    name: &'static str,
    glyph: &'static str,
    homepage: &'static str,
    /// The protocol in use, or the default before connecting.
    protocol: &'static str,
    /// Every protocol the agent can use; the first is the default.
    protocols: Vec<&'static str>,
    installed: bool,
    files: Vec<String>,
    connection: Option<ConnectionView>,
    /// `connected`, `edited` (the file no longer holds the key written here),
    /// `other` (points at the server but was not set up here) or
    /// `none`.
    status: &'static str,
    config_error: Option<String>,
    /// See `AgentId::notice`.
    notice: Option<&'static str>,
    /// See `Meta::after_write`.
    after_write: Option<&'static str>,
}

fn agent_view(inner: &Inner, id: AgentId) -> AgentView {
    let current = id.read(&inner.home);
    let connection = inner.local.connections.get(&id);
    let server = inner
        .local
        .server
        .as_deref()
        .map(|server| server.trim_end_matches('/'));
    let status = match (&current, connection) {
        (Ok(current), Some(connection)) => {
            if current
                .secret
                .as_deref()
                .is_some_and(|secret| secret.starts_with(&connection.key_prefix))
            {
                "connected"
            } else {
                "edited"
            }
        }
        (Ok(current), None)
            if current.base_url.as_deref().is_some_and(|url| {
                let url = bili::unwrap(url);
                server.is_some_and(|server| url.starts_with(server))
            }) =>
        {
            "other"
        }
        _ => "none",
    };
    AgentView {
        id,
        name: id.name(),
        glyph: id.meta().glyph,
        homepage: id.meta().homepage,
        protocol: id
            .protocol_or_default(connection.and_then(|c| c.protocol))
            .server_name(),
        protocols: id
            .meta()
            .protocols
            .iter()
            .map(|protocol| protocol.server_name())
            .collect(),
        // A connected agent stays on the page even if detection misses it.
        installed: connection.is_some() || id.installed(&inner.home),
        files: id
            .files(&inner.home)
            .iter()
            .map(|path| display_path(&inner.home, path))
            .collect(),
        connection: connection.map(|c| ConnectionView {
            server: c.server.clone(),
            key_id: c.key_id.clone(),
            key_name: c.key_name.clone(),
            key_prefix: c.key_prefix.clone(),
            model: c.model.clone(),
            connected_at: c.connected_at,
            last_test: c.last_test.clone(),
            can_undo: !c.last_backups.is_empty(),
            background_model: c.background_model.clone(),
            background_missing: c
                .background_model
                .as_ref()
                .is_some_and(|model| !c.models.is_empty() && !c.models.contains(model)),
            models_update: c.models_update.clone(),
        }),
        status,
        config_error: current.err().map(|error| error.to_string()),
        notice: id.notice(&inner.home),
        after_write: id.meta().after_write,
    }
}

/// `~/.codex/config.toml` reads better than an absolute path.
fn display_path(home: &std::path::Path, path: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[tauri::command]
async fn agents(app: tauri::State<'_, App>) -> Result<Vec<AgentView>> {
    let inner = app.0.lock().await;
    Ok(agents::ALL
        .iter()
        .map(|id| agent_view(&inner, *id))
        .collect())
}

#[derive(Serialize)]
struct PreviewView {
    changes: Vec<Change>,
    /// Connecting changes Claude Code's background model, so the window
    /// lets the user choose it.
    asks_background_model: bool,
}

#[tauri::command]
async fn preview(
    app: tauri::State<'_, App>,
    agent: AgentId,
    key_id: String,
    model: String,
    models: Vec<String>,
    background_model: Option<String>,
    protocol: Option<Protocol>,
) -> Result<PreviewView> {
    let inner = app.0.lock().await;
    let key = inner.saved(&key_id)?;
    let server = inner.server();
    let asks_background_model = agent.asks_background_model(&inner.home, &models)?;
    // The preview shows the key masked.
    let settings = Settings {
        server: &server,
        secret: &key.key_prefix,
        model: &model,
        models: &models,
        background_model: background_model
            .as_deref()
            .filter(|_| asks_background_model),
        protocol: Some(agent.protocol_or_default(protocol)),
    };
    Ok(PreviewView {
        changes: agent.preview(&inner.home, &settings)?,
        asks_background_model,
    })
}

#[tauri::command]
async fn apply(
    app: tauri::State<'_, App>,
    agent: AgentId,
    key_id: String,
    model: String,
    background_model: Option<String>,
    protocol: Option<Protocol>,
) -> Result<TestResult> {
    let protocol = agent.protocol_or_default(protocol);
    let mut inner = app.0.lock().await;
    let model = model.trim().to_owned();
    if model.is_empty() {
        return Err(CoreError::Other("请选择模型".into()));
    }
    let (saved, server, secret) = inner.secret(&key_id)?;
    let key = match Api::new(server.clone()).key_summary(&secret).await {
        Err(CoreError::Unauthorized) => {
            return Err(CoreError::Other("Key 已失效或被撤销，请换一把 Key".into()));
        }
        result => result?.as_client_key(),
    };
    if !key.enabled {
        return Err(CoreError::Other("这把 Key 已被管理员停用".into()));
    }

    let files = agent.files(&inner.home);
    let label = serde_json::to_value(agent)
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "agent".into());
    let backups = state::back_up(&files, &inner.data_dir, &label)?;
    let models = key_models(&key, protocol);
    let background_model = background_model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .filter(|_| {
            agent
                .asks_background_model(&inner.home, &models)
                .unwrap_or(false)
        });
    let settings = Settings {
        server: &server,
        secret: &secret,
        model: &model,
        models: &models,
        background_model: background_model.as_deref(),
        protocol: Some(protocol),
    };
    let mut prior = agent.apply(&inner.home, &settings)?;
    let previous = inner.local.connections.remove(&agent);
    if let Some(previous) = &previous {
        // Keep the user's own values from the first connect.
        for (field, value) in &previous.prior {
            prior.insert(field.clone(), value.clone());
        }
    }
    let result = probe::test(&server, protocol, &secret, &model).await;
    inner.local.connections.insert(
        agent,
        Connection {
            server: server.to_string(),
            key_id: key.id,
            key_name: saved.name().to_owned(),
            key_prefix: key.key_prefix,
            model,
            connected_at: chrono::Utc::now(),
            prior,
            last_backups: backups,
            last_test: Some(result.clone()),
            previous: previous.map(|mut previous| {
                previous.previous = None;
                Box::new(previous)
            }),
            background_model,
            models,
            models_update: None,
            protocol: (protocol != agent.protocol()).then_some(protocol),
        },
    );
    inner.local.save(&inner.data_dir)?;
    Ok(result)
}

fn key_models(key: &ClientKey, protocol: Protocol) -> Vec<String> {
    key.models
        .get(protocol.server_name())
        .map(|models| models.iter().cloned().collect())
        .unwrap_or_default()
}

/// The base URL each protocol's clients take, for tools set up by hand.
#[derive(Serialize)]
struct Endpoint {
    protocol: &'static str,
    base_url: String,
}

#[tauri::command]
async fn endpoints(app: tauri::State<'_, App>) -> Result<Vec<Endpoint>> {
    let server = app.0.lock().await.server();
    Ok([Protocol::OpenAi, Protocol::Anthropic, Protocol::Gemini]
        .into_iter()
        .map(|protocol| Endpoint {
            protocol: protocol.server_name(),
            base_url: agents::protocol_url(protocol, &server),
        })
        .collect())
}

fn set_clipboard(text: &str) -> Result<()> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| CoreError::Other(format!("无法写入剪贴板：{error}")))
}

#[tauri::command]
async fn copy_text(text: String) -> Result<()> {
    set_clipboard(&text)
}

/// Puts a key on the clipboard for a tool that cannot be set up here; the
/// secret never reaches the window.
#[tauri::command]
async fn copy_key(app: tauri::State<'_, App>, key_id: String) -> Result<()> {
    let (_, _, secret) = app.0.lock().await.secret(&key_id)?;
    set_clipboard(&secret)
}

/// Brings the model lists written into connected agents in line with
/// their keys' current models. Agents the user re-pointed, and keys that
/// cannot be read, are left as they are. Errors here never sign the user
/// out; that is left to what the user does in the window.
pub(crate) async fn sync_models(app: &App) {
    let _ = update_models(app, false).await;
}

/// Does the syncing; `manual` runs it with syncing turned off too. Returns
/// the agents whose files changed, or the first failure, after the agents
/// that could be updated are.
async fn update_models(app: &App, manual: bool) -> Result<Vec<&'static str>> {
    // The window stays usable while the server is asked.
    let (server, wanted) = {
        let inner = app.0.lock().await;
        if inner.local.connections.is_empty() || !(manual || inner.local.sync_models) {
            return Ok(Vec::new());
        }
        let mut wanted: Vec<(SavedKey, Option<String>)> = Vec::new();
        for connection in inner.local.connections.values() {
            if wanted.iter().any(|(key, _)| key.id == connection.key_id) {
                continue;
            }
            if let Some(key) = inner.local.key(&connection.key_id) {
                let secret = Credentials::for_key(&key.id, &inner.data_dir).load();
                wanted.push((key.clone(), secret));
            }
        }
        (inner.server(), wanted)
    };
    let mut keys = Vec::new();
    let mut failed = None;
    for (_, result) in summaries(server, wanted).await {
        match result {
            Ok(summary) => keys.push(summary.as_client_key()),
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    let mut inner = app.0.lock().await;
    if !(manual || inner.local.sync_models) {
        return Ok(Vec::new());
    }
    let mut synced = Vec::new();
    for (agent, connection) in &inner.local.connections {
        let Some(key) = keys
            .iter()
            .find(|key| key.id == connection.key_id && key.enabled)
        else {
            continue;
        };
        let connected = agent
            .read(&inner.home)
            .ok()
            .and_then(|current| current.secret)
            .is_some_and(|secret| secret.starts_with(&connection.key_prefix));
        let models = key_models(key, agent.protocol_or_default(connection.protocol));
        if !connected {
            continue;
        }
        match agent.sync_models(&inner.home, &connection.model, &models) {
            Ok(written) => synced.push((*agent, models, written)),
            Err(error) => {
                failed.get_or_insert(CoreError::Other(format!("{}：{error}", agent.name())));
            }
        }
    }
    let mut changed = false;
    let mut updated = Vec::new();
    for (agent, models, written) in synced {
        if let Some(connection) = inner.local.connections.get_mut(&agent) {
            let new = connection.models != models;
            changed |= new;
            if new || written {
                updated.push(agent.name());
            }
            connection.record_models(models);
        }
    }
    if changed {
        let _ = inner.local.save(&inner.data_dir);
    }
    failed.map_or(Ok(updated), Err)
}

/// The settings page's button: syncs now, with syncing on or off.
#[tauri::command]
async fn refresh_models(app: tauri::State<'_, App>) -> Result<Vec<&'static str>> {
    update_models(&app, true).await
}

#[tauri::command]
async fn sync_setting(app: tauri::State<'_, App>) -> Result<bool> {
    Ok(app.0.lock().await.local.sync_models)
}

/// Turning syncing on brings the lists up to date straight away.
#[tauri::command]
async fn set_sync_setting(app: tauri::State<'_, App>, on: bool) -> Result<bool> {
    {
        let mut inner = app.0.lock().await;
        inner.local.sync_models = on;
        inner.local.save(&inner.data_dir)?;
    }
    if on {
        sync_models(&app).await;
    }
    Ok(on)
}

#[tauri::command]
async fn test_agent(app: tauri::State<'_, App>, agent: AgentId) -> Result<TestResult> {
    let mut inner = app.0.lock().await;
    let current = agent.read(&inner.home)?;
    let (Some(secret), Some(model)) = (current.secret, current.model) else {
        return Err(CoreError::Other("该 Agent 尚未接入 OpenVAIR".into()));
    };
    let protocol = agent.protocol_or_default(
        inner
            .local
            .connections
            .get(&agent)
            .and_then(|connection| connection.protocol),
    );
    let server = inner.server();
    let result = probe::test(&server, protocol, &secret, &model).await;
    let data_dir = inner.data_dir.clone();
    if let Some(connection) = inner.local.connections.get_mut(&agent) {
        connection.last_test = Some(result.clone());
        inner.local.save(&data_dir)?;
    }
    Ok(result)
}

/// Removes OpenVAIR from the agent and puts back the user's earlier values.
#[tauri::command]
async fn disconnect(app: tauri::State<'_, App>, agent: AgentId) -> Result<()> {
    let mut inner = app.0.lock().await;
    let Some(connection) = inner.local.connections.get(&agent).cloned() else {
        return Ok(());
    };
    let label = format!(
        "{}-disconnect",
        agent.name().to_lowercase().replace(' ', "-")
    );
    state::back_up(&agent.files(&inner.home), &inner.data_dir, &label)?;
    agent.remove(&inner.home, &connection.prior)?;
    inner.local.connections.remove(&agent);
    inner.local.save(&inner.data_dir)
}

/// Opens bili's web interface. The address comes from the agents' settings,
/// never from the UI, and is always a loopback origin.
#[tauri::command]
async fn open_bili(app: tauri::State<'_, App>) -> Result<()> {
    let inner = app.0.lock().await;
    tray::open_dashboard(&inner.home, &inner.server())
}

#[tauri::command]
async fn bili_status(
    app: tauri::State<'_, App>,
    watch: tauri::State<'_, tray::Watch>,
) -> Result<bili::Status> {
    let inner = app.0.lock().await;
    Ok(watch.report(bili::status(&inner.home, &inner.server())?))
}

/// Settings are backed up first; bili commands can be slow, so they run
/// off the async runtime and without holding the app state.
async fn change_bili(
    app: &App,
    watch: &tray::Watch,
    agent: AgentId,
    on: bool,
) -> Result<bili::Status> {
    let change = match (agent, on) {
        (AgentId::ClaudeCode, true) => bili::enable,
        (AgentId::ClaudeCode, false) => bili::disable,
        (AgentId::Codex, true) => bili::codex::enable,
        (AgentId::Codex, false) => bili::codex::disable,
        _ => {
            return Err(CoreError::Other(format!(
                "{} 暂不支持长对话压缩",
                agent.name()
            )));
        }
    };
    let (home, server) = {
        let inner = app.0.lock().await;
        if !inner.local.connections.contains_key(&agent) {
            return Err(CoreError::Other(format!(
                "请先把 {} 接入 OpenVAIR",
                agent.name()
            )));
        }
        let label = format!(
            "{}-bili-{}",
            agent.name().to_lowercase().replace(' ', "-"),
            if on { "enable" } else { "disable" }
        );
        state::back_up(&agent.files(&inner.home), &inner.data_dir, &label)?;
        (inner.home.clone(), inner.server())
    };
    let upstream = agents::base_url(agent, &server);
    let change_home = home.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || change(&change_home, &upstream))
        .await
        .map_err(|error| CoreError::Other(error.to_string()))?;
    watch.check();
    outcome?;
    Ok(watch.report(bili::status(&home, &server)?))
}

/// Installing touches no agent settings, so it needs no backup or connection.
#[tauri::command]
async fn install_bili(app: tauri::State<'_, App>) -> Result<bili::Status> {
    let (home, server) = {
        let inner = app.0.lock().await;
        (inner.home.clone(), inner.server())
    };
    let install_home = home.clone();
    tauri::async_runtime::spawn_blocking(move || bili::install(&install_home))
        .await
        .map_err(|error| CoreError::Other(error.to_string()))??;
    bili::status(&home, &server)
}

#[tauri::command]
async fn enable_bili(
    app: tauri::State<'_, App>,
    watch: tauri::State<'_, tray::Watch>,
    agent: AgentId,
) -> Result<bili::Status> {
    change_bili(&app, &watch, agent, true).await
}

#[tauri::command]
async fn disable_bili(
    app: tauri::State<'_, App>,
    watch: tauri::State<'_, tray::Watch>,
    agent: AgentId,
) -> Result<bili::Status> {
    change_bili(&app, &watch, agent, false).await
}

/// Whether OpenVAIR Desktop starts, in the tray only, when the user logs in.
#[tauri::command]
async fn autostart(app: tauri::AppHandle) -> Result<bool> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| CoreError::Other(error.to_string()))
}

#[tauri::command]
async fn set_autostart(app: tauri::AppHandle, on: bool) -> Result<bool> {
    let launcher = app.autolaunch();
    if on {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .and_then(|()| launcher.is_enabled())
    .map_err(|error| CoreError::Other(format!("设置开机启动失败：{error}")))
}

/// Restores the files from before the latest write, e.g. after a failed test.
#[tauri::command]
async fn undo(app: tauri::State<'_, App>, agent: AgentId) -> Result<()> {
    let mut inner = app.0.lock().await;
    let Some(connection) = inner.local.connections.remove(&agent) else {
        return Ok(());
    };
    state::restore(&connection.last_backups)?;
    if let Some(previous) = connection.previous {
        inner.local.connections.insert(agent, *previous);
    }
    inner.local.save(&inner.data_dir)
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    backups_dir: String,
}

#[tauri::command]
async fn app_info(app: tauri::State<'_, App>) -> Result<AppInfo> {
    let inner = app.0.lock().await;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        backups_dir: display_path(&inner.home, &state::backups_dir(&inner.data_dir)),
    })
}

#[tauri::command]
async fn open_backups(app: tauri::State<'_, App>) -> Result<()> {
    let inner = app.0.lock().await;
    let directory = state::backups_dir(&inner.data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| CoreError::file(&directory, error))?;
    open::that_detached(&directory).map_err(|error| CoreError::Other(error.to_string()))
}

/// Opens a page of the server, such as the key holder's usage page.
#[tauri::command]
async fn open_console(app: tauri::State<'_, App>, path: String) -> Result<()> {
    let inner = app.0.lock().await;
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(CoreError::Other("invalid path".into()));
    }
    open::that_detached(Api::new(inner.server()).url(&path))
        .map_err(|error| CoreError::Other(error.to_string()))
}

/// Opens an HTTPS page such as an agent's install guide.
#[tauri::command]
async fn open_external(url: String) -> Result<()> {
    let parsed = url::Url::parse(&url).map_err(|_| CoreError::Other("invalid url".into()))?;
    if parsed.scheme() != "https" {
        return Err(CoreError::Other("invalid url".into()));
    }
    open::that_detached(parsed.as_str()).map_err(|error| CoreError::Other(error.to_string()))
}

pub fn run() {
    let data_dir = state::data_dir();
    let local = LocalState::load(&data_dir);
    let inner = Inner {
        home: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
        data_dir,
        local,
    };
    let hidden = std::env::args().any(|argument| argument == tray::HIDDEN);
    tauri::Builder::default()
        // A second launch shows the running app, which may only be in the tray.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![tray::HIDDEN]),
        ))
        .manage(App(Mutex::new(inner)))
        .setup(move |app| {
            tray::setup(app)?;
            if hidden {
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                tray::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                tray::hide(window);
            }
        })
        .invoke_handler(tauri::generate_handler![
            session,
            keys,
            add_key,
            check_server,
            rename_key,
            set_default_key,
            remove_key,
            change_server,
            key_summary,
            agents,
            preview,
            apply,
            test_agent,
            disconnect,
            undo,
            bili_status,
            open_bili,
            install_bili,
            enable_bili,
            disable_bili,
            autostart,
            set_autostart,
            sync_setting,
            set_sync_setting,
            refresh_models,
            app_info,
            open_backups,
            open_console,
            open_external,
            endpoints,
            copy_text,
            copy_key,
        ])
        .build(tauri::generate_context!())
        .expect("error while building OpenVAIR Desktop")
        .run(|app, event| {
            // Opening the app again from Finder while it is in the tray.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
