use super::*;
use crate::{crypto::Crypto, gateway};
use axum::{Router, body::Body, body::to_bytes, http::Request, routing::post};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, Notify};

/// (Authorization header, model) pairs received by the mock upstream.
type Seen = Arc<std::sync::Mutex<Vec<(String, String)>>>;

struct Fixture {
    state: AppState,
    seen: Seen,
    pause: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(db: PgPool) -> Self {
        let seen = Seen::default();
        let pause = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let mock = Router::new().route(
            "/v1/chat/completions",
            post({
                let seen = seen.clone();
                let pause = pause.clone();
                let entered = entered.clone();
                let release = release.clone();
                move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = seen.clone();
                    let pause = pause.clone();
                    let entered = entered.clone();
                    let release = release.clone();
                    async move {
                        let auth = headers["authorization"].to_str().unwrap().to_owned();
                        let model = body["model"].as_str().unwrap().to_owned();
                        seen.lock().unwrap().push((auth, model.clone()));
                        if pause.swap(false, Ordering::SeqCst) {
                            entered.notify_one();
                            release.notified().await;
                        }
                        Json(json!({
                            "id": "test", "object": "chat.completion", "model": model,
                            "choices": [{"message": {"content": "WRONG"}, "finish_reason": "stop"}],
                            "usage": {"prompt_tokens": 3, "completion_tokens": 5}
                        }))
                    }
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        // Model lists and default check models come from the global catalog.
        sqlx::query(
            "INSERT INTO model_catalog(pattern,label) VALUES ('gpt-a','gpt-a'),('gpt-b','gpt-b')
             ON CONFLICT (pattern) DO NOTHING",
        )
        .execute(&db)
        .await
        .unwrap();
        let state = AppState {
            db,
            redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
            // Public upstream URLs pass the SSRF check; the proxy sends them to the mock.
            http: reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(proxy_url).unwrap())
                .build()
                .unwrap(),
            crypto: Arc::new(Crypto::from_env().unwrap()),
            semaphores: Arc::new(Mutex::new(HashMap::new())),
            cookie_secure: false,
            super_admin_emails: Arc::default(),
            redis_connection: Default::default(),
            routing_settings: Default::default(),
            prices: Default::default(),
        };
        Self {
            state,
            seen,
            pause,
            entered,
            release,
            server,
        }
    }

    fn db(&self) -> &PgPool {
        &self.state.db
    }

    async fn account(&self, name: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(self.db())
            .await
            .unwrap()
    }

    async fn upstream(&self, account: Uuid, model: &str, secret: &str, priority: i32) -> Uuid {
        let channel: Uuid = sqlx::query_scalar(
            "INSERT INTO channels(account_id,name,protocol,base_url,models,priority)
             VALUES ($1,'upstream','openai','http://1.1.1.1/v1',$2,$3) RETURNING id",
        )
        .bind(account)
        .bind(json!([model]))
        .bind(priority)
        .fetch_one(self.db())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,'test',$2)",
        )
        .bind(channel)
        .bind(self.state.crypto.encrypt(secret).unwrap())
        .execute(self.db())
        .await
        .unwrap();
        channel
    }

    async fn key(&self, account: Uuid, raw: &str, allowed: Value) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models)
             VALUES ($1,'test key','vair_test',$2,$3) RETURNING id",
        )
        .bind(account)
        .bind(auth::hash_token(raw))
        .bind(allowed)
        .fetch_one(self.db())
        .await
        .unwrap()
    }

    async fn relay(&self, account: Uuid, source_key: Uuid, priority: i32, mapping: Value) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO channels(account_id,name,protocol,base_url,priority,model_mapping,
                 model_mode,kind,source_key_id)
             VALUES ($1,'relay','any','',$2,$3,'passthrough','vair_key',$4) RETURNING id",
        )
        .bind(account)
        .bind(priority)
        .bind(mapping)
        .bind(source_key)
        .fetch_one(self.db())
        .await
        .unwrap()
    }

    /// An owner session in `account` for `email`.
    async fn session(&self, account: Uuid, email: &str) -> HeaderMap {
        let user: Uuid = sqlx::query_scalar(
            "INSERT INTO users(email,password_hash) VALUES ($1,'test-only') RETURNING id",
        )
        .bind(email)
        .fetch_one(self.db())
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships(account_id,user_id,role) VALUES ($1,$2,'owner')")
            .bind(account)
            .bind(user)
            .execute(self.db())
            .await
            .unwrap();
        let token = format!("session-{user}");
        sqlx::query(
            "INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')",
        )
        .bind(user)
        .bind(auth::hash_token(&token))
        .execute(self.db())
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            format!("gateway_session={token}").parse().unwrap(),
        );
        headers
    }

    async fn call(&self, method: &str, path: &str, key: &str, body: Option<Value>) -> (u16, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {key}"))
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
            .unwrap();
        let response = gateway::handle(State(self.state.clone()), request).await;
        let status = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn chat(&self, key: &str, model: &str) -> u16 {
        self.call(
            "POST",
            "/v1/chat/completions",
            key,
            Some(json!({"model": model, "messages": []})),
        )
        .await
        .0
    }

    fn last_seen(&self) -> Option<(String, String)> {
        self.seen.lock().unwrap().last().cloned()
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relay_probe_transport_errors_hide_source_details(db: PgPool) {
    let mut f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!([])).await;
    let caller = f.account("caller").await;
    let headers = f.session(caller, "caller@example.invalid").await;
    let relay = f.relay(caller, source, 100, json!({})).await;

    // Judged unavailable on the first failure, so health carries the message.
    sqlx::query(
        "INSERT INTO channel_monitor_configs(channel_id,availability_failure_threshold) VALUES ($1,1) ON CONFLICT(channel_id) DO UPDATE SET availability_failure_threshold=1",
    )
    .bind(relay)
    .execute(f.db())
    .await
    .unwrap();
    // A refused connection makes reqwest include the source URL in its error.
    f.server.abort();
    assert!((&mut f.server).await.unwrap_err().is_cancelled());
    let Json(response) = crate::check_channel(State(f.state.clone()), Path(relay), headers)
        .await
        .unwrap();
    assert_eq!(response["healthy"], false);
    let message = response["message"].as_str().unwrap();
    assert!(message.starts_with("upstream request failed:"), "{message}");
    for private in [
        "1.1.1.1",
        "/v1/chat/completions",
        "owner-upstream-test-only",
    ] {
        assert!(!response.to_string().contains(private), "{response}");
    }

    // Both persisted views must carry the same sanitized message.
    let (status, health_error): (String, String) =
        sqlx::query_as("SELECT status,last_error FROM channel_health WHERE channel_id=$1")
            .bind(relay)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(status, "unhealthy");
    assert_eq!(health_error, message);
    let (error, details): (String, Value) = sqlx::query_as(
        "SELECT error_code,details FROM channel_monitor_runs WHERE channel_id=$1 AND kind='availability'",
    ).bind(relay).fetch_one(f.db()).await.unwrap();
    assert_eq!(error, message);
    assert_eq!(details, json!({"message": message}));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn replacement_resets_quality_results_and_cancels_queued_jobs(db: PgPool) {
    use crate::{
        monitor,
        quality_monitors::{self as qm, CreateInput, Settings},
    };
    let f = Fixture::new(db).await;
    let old_owner = f.account("old owner").await;
    f.upstream(old_owner, "gpt-a", "old-upstream-test-only", 100)
        .await;
    let old_key = f.key(old_owner, "vair_old_test_only", json!([])).await;
    let new_owner = f.account("new owner").await;
    f.upstream(new_owner, "gpt-a", "new-upstream-test-only", 100)
        .await;
    f.key(new_owner, "vair_new_test_only", json!([])).await;
    let caller = f.account("caller").await;
    f.key(caller, "vair_caller_test_only", json!([])).await;
    let headers = f.session(caller, "caller@example.invalid").await;
    let relay = f.relay(caller, old_key, 100, json!({})).await;
    let settings = Settings {
        schedule_enabled: true,
        interval_seconds: 900,
        methods: vec!["tools".into()],
        failure_threshold: 1,
        route_policy: "exclude".into(),
        ..Settings::default()
    };
    let (id, _) = qm::insert(
        f.db(),
        caller,
        &CreateInput {
            channel_id: relay,
            model: "gpt-a".into(),
            settings: settings.clone(),
            run_immediately: Some(false),
            kind: "service".into(),
            authenticity: None,
        },
    )
    .await
    .unwrap();
    let job = qm::enqueue(f.db(), caller, id, "manual").await.unwrap();
    assert_eq!(monitor::claim_job(&f.state).await.unwrap(), Some(job));
    monitor::execute_job(&f.state, job).await.unwrap();
    let before = qm::load(f.db(), caller).await.unwrap().remove(0);
    assert_eq!(before.status, "degraded");
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 404);
    let queued = qm::enqueue(f.db(), caller, id, "manual").await.unwrap();

    let _ = replace_key(
        State(f.state.clone()),
        Path(relay),
        headers,
        Json(ResolveInput {
            secret: "vair_new_test_only".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(
        f.last_seen(),
        Some(("Bearer new-upstream-test-only".into(), "gpt-a".into()))
    );
    let after = qm::load(f.db(), caller).await.unwrap().remove(0);
    assert_eq!(after.id, id);
    assert_eq!(after.config_version, before.config_version + 1);
    assert_eq!(after.status, "unknown");
    assert_eq!(after.last_observation, "unknown");
    assert_eq!(after.consecutive_failures, 0);
    assert!(after.score.is_none() && after.valid_until.is_none() && after.checked_at.is_none());
    assert!(after.last_error.is_none() && after.schedule_error.is_none());
    assert_eq!(after.schedule_enabled, settings.schedule_enabled);
    assert_eq!(after.interval_seconds, settings.interval_seconds);
    assert_eq!(after.methods, json!(settings.methods));
    assert_eq!(after.failure_threshold, settings.failure_threshold);
    assert_eq!(after.route_policy, settings.route_policy);
    assert!(after.next_run_at.is_some());
    let (status, reason): (String, String) =
        sqlx::query_as("SELECT status,error_code FROM quality_monitor_jobs WHERE id=$1")
            .bind(queued)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), reason.as_str()),
        ("cancelled", "channel_changed")
    );
    assert!(monitor::claim_job(&f.state).await.unwrap().is_none());
    let history: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_monitor_runs WHERE monitor_id=$1")
            .bind(id)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(history, 1);
    // Existing scheduling preferences still trigger a fresh run for the new key.
    monitor::schedule_due(&f.state).await.unwrap();
    assert!(monitor::claim_job(&f.state).await.unwrap().is_some());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn replacement_prevents_running_quality_job_from_applying_old_results(db: PgPool) {
    use crate::{
        monitor,
        quality_monitors::{self as qm, CreateInput, Settings},
    };
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let old_key = f.key(owner, "vair_old_test_only", json!([])).await;
    f.key(owner, "vair_new_test_only", json!([])).await;
    let caller = f.account("caller").await;
    f.key(caller, "vair_caller_test_only", json!([])).await;
    let headers = f.session(caller, "caller@example.invalid").await;
    let relay = f.relay(caller, old_key, 100, json!({})).await;
    let (id, _) = qm::insert(
        f.db(),
        caller,
        &CreateInput {
            channel_id: relay,
            model: "gpt-a".into(),
            settings: Settings {
                methods: vec!["tools".into()],
                failure_threshold: 1,
                route_policy: "exclude".into(),
                ..Settings::default()
            },
            run_immediately: Some(false),
            kind: "service".into(),
            authenticity: None,
        },
    )
    .await
    .unwrap();
    let job = qm::enqueue(f.db(), caller, id, "manual").await.unwrap();
    assert_eq!(monitor::claim_job(&f.state).await.unwrap(), Some(job));
    f.pause.store(true, Ordering::SeqCst);
    let state = f.state.clone();
    let running = tokio::spawn(async move { monitor::execute_job(&state, job).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), f.entered.notified())
        .await
        .unwrap();
    let _ = replace_key(
        State(f.state.clone()),
        Path(relay),
        headers,
        Json(ResolveInput {
            secret: "vair_new_test_only".into(),
        }),
    )
    .await
    .unwrap();
    f.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let after = qm::load(f.db(), caller).await.unwrap().remove(0);
    assert_eq!(after.status, "unknown");
    assert!(after.score.is_none() && after.valid_until.is_none() && after.next_run_at.is_none());
    assert_eq!(after.job_status.as_deref(), Some("cancelled"));
    let (observation, details): (String, Value) =
        sqlx::query_as("SELECT status,details FROM channel_monitor_runs WHERE monitor_id=$1")
            .bind(id)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(observation, "degraded");
    assert_eq!(details["applied"], false);
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relays_through_source_key_and_records_both_sides(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    let upstream = f
        .upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!(["gpt-a"])).await;
    let caller = f.account("caller").await;
    let caller_key = f.key(caller, "vair_caller_test_only", json!([])).await;
    let relay = f
        .relay(caller, source, 100, json!({"team-chat": "gpt-a"}))
        .await;

    assert_eq!(f.chat("vair_caller_test_only", "team-chat").await, 200);
    assert_eq!(
        f.last_seen(),
        Some(("Bearer owner-upstream-test-only".into(), "gpt-a".into()))
    );

    #[derive(FromRow)]
    struct Event {
        id: Uuid,
        account_id: Uuid,
        virtual_key_id: Uuid,
        channel_id: Option<Uuid>,
        model: String,
        input_tokens: Option<i64>,
        relayed_event_id: Option<Uuid>,
    }
    let rows: Vec<Event> = sqlx::query_as(
        "SELECT id,account_id,virtual_key_id,channel_id,model,input_tokens,relayed_event_id
         FROM usage_events ORDER BY created_at,relayed_event_id NULLS FIRST",
    )
    .fetch_all(f.db())
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    let (served, relayed) = (&rows[0], &rows[1]);
    assert_eq!(
        (
            served.account_id,
            served.virtual_key_id,
            served.channel_id,
            served.model.as_str(),
            served.input_tokens,
            served.relayed_event_id,
        ),
        (owner, source, Some(upstream), "gpt-a", Some(3), None)
    );
    assert_eq!(
        (
            relayed.account_id,
            relayed.virtual_key_id,
            relayed.channel_id,
            relayed.model.as_str(),
            relayed.input_tokens,
            relayed.relayed_event_id,
        ),
        (
            caller,
            caller_key,
            Some(relay),
            "team-chat",
            Some(3),
            Some(served.id)
        )
    );

    let (status, catalog) = f
        .call("GET", "/v1/models", "vair_caller_test_only", None)
        .await;
    assert_eq!(status, 200);
    let ids: Vec<&str> = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["gpt-a", "team-chat"]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn unavailable_source_keys_fail_over_and_relays_do_not_loop(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!([])).await;
    let caller = f.account("caller").await;
    let caller_key = f.key(caller, "vair_caller_test_only", json!([])).await;
    f.relay(caller, source, 10, json!({})).await;
    let fallback = f
        .upstream(caller, "gpt-a", "caller-upstream-test-only", 200)
        .await;

    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer owner-upstream-test-only");

    // A paused source key makes the relay unavailable rather than failing the request.
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer caller-upstream-test-only");

    // So does a model outside the source key's permissions.
    sqlx::query(
        "UPDATE virtual_api_keys SET enabled=true, allowed_models='[\"other\"]' WHERE id=$1",
    )
    .bind(source)
    .execute(f.db())
    .await
    .unwrap();
    f.seen.lock().unwrap().clear();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer caller-upstream-test-only");

    // Two accounts relaying to each other cannot loop: a path visits each
    // account at most once.
    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(fallback)
        .execute(f.db())
        .await
        .unwrap();
    sqlx::query("UPDATE virtual_api_keys SET model_mode='all', allowed_models='[]' WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    f.relay(owner, caller_key, 10, json!({})).await;
    f.seen.lock().unwrap().clear();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-missing").await, 404);
    assert_eq!(f.chat("vair_owner_test_only", "gpt-missing").await, 404);
    assert!(f.last_seen().is_none());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn pasted_keys_control_routing(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("Owner Lab").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!(["gpt-a"])).await;
    let owner_headers = f.session(owner, "owner@example.invalid").await;
    let caller = f.account("caller").await;
    f.key(caller, "vair_caller_test_only", json!([])).await;
    let caller_headers = f.session(caller, "caller@example.invalid").await;
    let state = || State(f.state.clone());

    // Pasting: own keys and unknown keys are rejected; a foreign key resolves to
    // its owner, label and reachable models.
    assert!(matches!(
        resolve(
            state(),
            caller_headers.clone(),
            Json(ResolveInput {
                secret: "vair_caller_test_only".into(),
            }),
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        resolve(
            state(),
            caller_headers.clone(),
            Json(ResolveInput {
                secret: "vair_unknown_test_only".into(),
            }),
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let Json(resolved) = resolve(
        state(),
        caller_headers.clone(),
        Json(ResolveInput {
            secret: " vair_owner_test_only ".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        (resolved.account_name.as_str(), resolved.models.as_slice()),
        ("Owner Lab", ["gpt-a".to_owned()].as_slice())
    );
    let paste = || {
        create_channel(
            state(),
            caller_headers.clone(),
            Json(CreateChannelInput {
                name: "Pasted".into(),
                secret: "vair_owner_test_only".into(),
                priority: None,
                model_mapping: None,
            }),
        )
    };
    let Json(created) = paste().await.unwrap();
    let pasted_channel: Uuid = serde_json::from_value(created["id"].clone()).unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    // Pasting the same key twice in one account is refused.
    assert!(matches!(paste().await, Err(AppError::Conflict(_))));

    let Json(keys) = crate::list_keys(state(), owner_headers.clone())
        .await
        .unwrap();
    assert_eq!(keys[0].channel_count, 1);

    // A recipient can check a VAIR key channel; the probe goes through the source key.
    let Json(check) = crate::check_channel(state(), Path(pasted_channel), caller_headers.clone())
        .await
        .unwrap();
    assert_eq!(check["healthy"], true);
    // It still has no upstream model list of its own to sync.
    assert!(matches!(
        crate::sync_channel_models(state(), Path(pasted_channel), caller_headers.clone()).await,
        Err(AppError::BadRequest(_))
    ));

    // Revoking the source key ends every channel built on it.
    crate::revoke_key(state(), Path(source), owner_headers)
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 404);
    let Json(channels) = crate::list_channels(state(), caller_headers).await.unwrap();
    assert_eq!(channels[0].source.as_ref().unwrap()["status"], "revoked");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn pasted_channels_can_replace_their_key(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let old_key = f.key(owner, "vair_old_test_only", json!([])).await;
    let new_key = f.key(owner, "vair_new_test_only", json!([])).await;
    f.key(owner, "vair_second_test_only", json!([])).await;
    let owner_headers = f.session(owner, "owner@example.invalid").await;
    let caller = f.account("caller").await;
    f.key(caller, "vair_caller_test_only", json!([])).await;
    let caller_headers = f.session(caller, "caller@example.invalid").await;
    let state = || State(f.state.clone());
    let replace = |channel: Uuid, secret: &str| {
        replace_key(
            state(),
            Path(channel),
            caller_headers.clone(),
            Json(ResolveInput {
                secret: secret.into(),
            }),
        )
    };

    let Json(created) = create_channel(
        state(),
        caller_headers.clone(),
        Json(CreateChannelInput {
            name: "Pasted".into(),
            secret: "vair_old_test_only".into(),
            priority: Some(7),
            model_mapping: Some(json!({"team-chat": "gpt-a"})),
        }),
    )
    .await
    .unwrap();
    let pasted: Uuid = serde_json::from_value(created["id"].clone()).unwrap();
    assert!(
        gateway::probe_channel(&f.state, caller, pasted)
            .await
            .unwrap()
            .0
    );

    // Own and unknown keys are refused as on creation.
    assert!(matches!(
        replace(pasted, "vair_caller_test_only").await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        replace(pasted, "vair_unknown_test_only").await,
        Err(AppError::BadRequest(_))
    ));

    // Replacing keeps the channel's settings and drops results of the old key.
    let _ = replace(pasted, "vair_new_test_only").await.unwrap();
    let (source, priority, mapping): (Uuid, i32, Value) =
        sqlx::query_as("SELECT source_key_id,priority,model_mapping FROM channels WHERE id=$1")
            .bind(pasted)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(
        (source, priority, mapping),
        (new_key, 7, json!({"team-chat": "gpt-a"}))
    );
    let health: Option<String> =
        sqlx::query_scalar("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(pasted)
            .fetch_optional(f.db())
            .await
            .unwrap();
    assert_eq!(health, None);
    crate::revoke_key(state(), Path(old_key), owner_headers.clone())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "team-chat").await, 200);

    // A key already used by another channel in the account is refused.
    let _ = create_channel(
        state(),
        caller_headers.clone(),
        Json(CreateChannelInput {
            name: "Second".into(),
            secret: "vair_second_test_only".into(),
            priority: None,
            model_mapping: None,
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        replace(pasted, "vair_second_test_only").await,
        Err(AppError::Conflict(_))
    ));

    // Only VAIR key channels take a pasted key, and a missing channel is 404.
    let upstream = f
        .upstream(caller, "gpt-c", "caller-upstream-test-only", 100)
        .await;
    assert!(matches!(
        replace(upstream, "vair_new_test_only").await,
        Err(AppError::BadRequest(message)) if message.contains("只有 VAIR Key")
    ));
    assert!(matches!(
        replace(Uuid::new_v4(), "vair_new_test_only").await,
        Err(AppError::NotFound)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn migration_removes_channels_created_by_key_grants(db: PgPool) {
    // Build the schema as it was before 0045, with channels from every origin.
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|migration| migration.version < 45)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&db).await.unwrap();
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    let source = f.key(owner, "vair_granted_test_only", json!([])).await;
    let replacement = f.key(owner, "vair_replacement_test_only", json!([])).await;
    let mut channels = HashMap::new();
    for (name, via, replaced) in [
        ("shared", Some("share"), false),
        ("granted", Some("grant"), false),
        ("granted-then-pasted", Some("grant"), true),
        ("pasted", Some("key"), false),
        ("unaudited", None, false),
    ] {
        let account = f.account(name).await;
        let key = if replaced { replacement } else { source };
        let channel = f.relay(account, key, 100, json!({})).await;
        if let Some(via) = via {
            sqlx::query(
                "INSERT INTO audit_logs(account_id,action,resource_type,resource_id,metadata)
                 VALUES ($1,'channel.created','channel',$2,$3)",
            )
            .bind(account)
            .bind(channel)
            .bind(json!({"kind": "vair_key", "via": via}))
            .execute(f.db())
            .await
            .unwrap();
        }
        if replaced {
            sqlx::query(
                "INSERT INTO audit_logs(account_id,action,resource_type,resource_id,metadata)
                 VALUES ($1,'channel.updated','channel',$2,$3)",
            )
            .bind(account)
            .bind(channel)
            .bind(json!({"source_key_replaced": true}))
            .execute(f.db())
            .await
            .unwrap();
        }
        channels.insert(name, (account, channel));
    }

    sqlx::migrate!("./migrations").run(f.db()).await.unwrap();

    let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM channels ORDER BY id")
        .fetch_all(f.db())
        .await
        .unwrap();
    for (name, (account, channel)) in &channels {
        let removed = matches!(*name, "shared" | "granted");
        assert_eq!(!remaining.contains(channel), removed, "{name}");
        let logged: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM audit_logs WHERE account_id=$1 AND resource_id=$2
             AND action='channel.deleted' AND metadata->>'reason'='key_grant_removed')",
        )
        .bind(account)
        .bind(channel)
        .fetch_one(f.db())
        .await
        .unwrap();
        assert_eq!(logged, removed, "{name}");
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relay_checks_prefer_the_model_the_source_key_last_served(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    f.upstream(owner, "gpt-b", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!([])).await;
    let caller = f.account("caller").await;
    let relay = f.relay(caller, source, 100, json!({})).await;
    let served = |model: &'static str, status: i32, seconds: i32| {
        sqlx::query(
            "INSERT INTO usage_events(account_id,virtual_key_id,model,protocol,status,duration_ms,created_at)
             VALUES ($1,$2,$3,'openai',$4,10,now()+make_interval(secs=>$5))",
        )
        .bind(owner)
        .bind(source)
        .bind(model)
        .bind(status)
        .bind(seconds)
        .execute(f.db())
    };

    // Without history the first reachable model is checked.
    assert!(
        gateway::probe_channel(&f.state, caller, relay)
            .await
            .unwrap()
            .0
    );
    assert_eq!(f.last_seen().unwrap().1, "gpt-a");

    // Later the source key serves gpt-b, and a gpt-a request fails.
    served("gpt-b", 200, 1).await.unwrap();
    served("gpt-a", 404, 2).await.unwrap();
    assert!(
        gateway::probe_channel(&f.state, caller, relay)
            .await
            .unwrap()
            .0
    );
    assert_eq!(f.last_seen().unwrap().1, "gpt-b");

    // A served model the key can no longer reach is not picked.
    sqlx::query("UPDATE virtual_api_keys SET allowed_models='[\"gpt-a\"]',model_mode='allowlist' WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    assert!(
        gateway::probe_channel(&f.state, caller, relay)
            .await
            .unwrap()
            .0
    );
    assert_eq!(f.last_seen().unwrap().1, "gpt-a");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn monitoring_through_source_key_counts_toward_its_limits(db: PgPool) {
    use crate::{
        monitor,
        quality_monitors::{self as qm, CreateInput, Settings},
    };
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!([])).await;
    // Each mock response reports 3 input + 5 output tokens.
    sqlx::query("UPDATE virtual_api_keys SET token_limit_daily=30 WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    let caller = f.account("caller").await;
    f.key(caller, "vair_caller_test_only", json!([])).await;
    let relay = f.relay(caller, source, 100, json!({})).await;
    let monitor_usage = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM usage_events
             WHERE account_id=$1 AND virtual_key_id=$2 AND source='monitor' AND input_tokens=3",
        )
        .bind(owner)
        .bind(source)
        .fetch_one(f.db())
        .await
        .unwrap()
    };

    // A manual check goes through the owner's upstream and is billed to the source key.
    let (healthy, _) = gateway::probe_channel(&f.state, caller, relay)
        .await
        .unwrap();
    assert!(healthy);
    assert_eq!(
        f.last_seen(),
        Some(("Bearer owner-upstream-test-only".into(), "gpt-a".into()))
    );
    assert_eq!(monitor_usage().await, 1);

    // Every quality test request is checked and recorded separately.
    let (monitor_id, _) = qm::insert(
        f.db(),
        caller,
        &CreateInput {
            channel_id: relay,
            model: "gpt-a".into(),
            settings: Settings {
                // Two requests: the integrity check asks twice to catch replays.
                methods: vec!["integrity".into()],
                ..Settings::default()
            },
            run_immediately: Some(false),
            kind: "service".into(),
            authenticity: None,
        },
    )
    .await
    .unwrap();
    let job = qm::enqueue(f.db(), caller, monitor_id, "manual")
        .await
        .unwrap();
    assert_eq!(monitor::claim_job(&f.state).await.unwrap(), Some(job));
    monitor::execute_job(&f.state, job).await.unwrap();
    assert_eq!(monitor_usage().await, 3);

    // 24 tokens used; one more check reaches the 30-token daily cap.
    assert!(
        gateway::probe_channel(&f.state, caller, relay)
            .await
            .unwrap()
            .0
    );
    let (healthy, reason) = gateway::probe_channel(&f.state, caller, relay)
        .await
        .unwrap();
    assert!(!healthy);
    assert!(reason.contains("上限"), "{reason}");
    assert_eq!(monitor_usage().await, 4);
    // The probe never reached the upstream, so it keeps the verdict rather than
    // judging the channel unhealthy: requests are refused by the cap itself.
    let status: (String, Option<String>) =
        sqlx::query_as("SELECT status,last_observation FROM channel_health WHERE channel_id=$1")
            .bind(relay)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(status, ("healthy".to_owned(), Some("unknown".to_owned())));
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 429);
    // Once the owner raises the cap the channel routes again without a re-check.
    sqlx::query("UPDATE virtual_api_keys SET token_limit_daily=NULL WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);

    // An upstream failure behind the key does count toward the channel's
    // availability, as for upstream channels; here one failure is the threshold.
    sqlx::query(
        "INSERT INTO channel_monitor_configs(channel_id,availability_failure_threshold) VALUES ($1,1) ON CONFLICT(channel_id) DO UPDATE SET availability_failure_threshold=1",
    )
    .bind(relay)
    .execute(f.db())
    .await
    .unwrap();
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/missing' WHERE account_id=$1")
        .bind(owner)
        .execute(f.db())
        .await
        .unwrap();
    let (healthy, reason) = gateway::probe_channel(&f.state, caller, relay)
        .await
        .unwrap();
    assert!(!healthy);
    assert!(reason.contains("HTTP 404"), "{reason}");
    let status: String =
        sqlx::query_scalar("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(relay)
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(status, "unhealthy");
    // Its only channel is still tried as a last resort, so the upstream error surfaces.
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 404);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relays_respect_key_scopes_spend_caps_and_cost_policy(db: PgPool) {
    let f = Fixture::new(db).await;
    // 3 input + 5 output tokens at $1000 per million tokens: $0.008 per request.
    sqlx::query(
        "UPDATE model_catalog SET input_per_1m=1000,output_per_1m=1000,cached_read_per_1m=0 WHERE pattern='gpt-a'",
    )
    .execute(f.db())
    .await
    .unwrap();
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-first-test-only", 100)
        .await;
    let allowed = f
        .upstream(owner, "gpt-a", "owner-second-test-only", 200)
        .await;
    f.upstream(owner, "gpt-b", "owner-unpriced-test-only", 100)
        .await;
    let source = f.key(owner, "vair_owner_test_only", json!([])).await;
    let caller = f.account("caller").await;
    let caller_key = f.key(caller, "vair_caller_test_only", json!([])).await;
    let relay = f.relay(caller, source, 100, json!({})).await;
    let caller_headers = f.session(caller, "caller@example.invalid").await;
    let status = || async {
        let Json(channels) = crate::list_channels(State(f.state.clone()), caller_headers.clone())
            .await
            .unwrap();
        channels[0].source.as_ref().unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // The source key's channel scope limits which of the owner's channels serve it.
    sqlx::query("UPDATE virtual_api_keys SET channel_mode='selected' WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO virtual_key_channels(virtual_key_id,channel_id) VALUES ($1,$2)")
        .bind(source)
        .bind(allowed)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer owner-second-test-only");
    // Out-of-scope models are not reachable or listed either.
    assert_eq!(f.chat("vair_caller_test_only", "gpt-b").await, 404);

    // A lifetime spend cap stops the relay; resetting the total window restores it.
    sqlx::query("UPDATE virtual_api_keys SET cost_limit_total_usd=0.01 WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(status().await, "exhausted_total");
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 429);
    sqlx::query("UPDATE virtual_api_keys SET usage_reset_total_at=now() WHERE id=$1")
        .bind(source)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(status().await, "active");
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);

    // A daily spend cap is reported separately because it recovers by itself.
    sqlx::query(
        "UPDATE virtual_api_keys SET cost_limit_total_usd=NULL,cost_limit_daily_usd=0.005 WHERE id=$1",
    )
    .bind(source)
    .execute(f.db())
    .await
    .unwrap();
    assert_eq!(status().await, "exhausted_daily");
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 429);
    sqlx::query(
        "UPDATE virtual_api_keys SET cost_limit_daily_usd=NULL,channel_mode='all' WHERE id=$1",
    )
    .bind(source)
    .execute(f.db())
    .await
    .unwrap();

    // An unpriced model is still relayed; its cost just stays unknown.
    assert_eq!(f.chat("vair_caller_test_only", "gpt-b").await, 200);

    // The caller key's own channel scope decides whether the relay channel is used.
    sqlx::query("UPDATE virtual_api_keys SET channel_mode='selected' WHERE id=$1")
        .bind(caller_key)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 404);
    sqlx::query("INSERT INTO virtual_key_channels(virtual_key_id,channel_id) VALUES ($1,$2)")
        .bind(caller_key)
        .bind(relay)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relays_chain_through_several_accounts(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    let upstream = f
        .upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let owner_key = f.key(owner, "vair_owner_test_only", json!([])).await;
    let agent = f.account("agent").await;
    let agent_relay = f
        .relay(agent, owner_key, 100, json!({"agent-chat": "gpt-a"}))
        .await;
    let agent_key = f.key(agent, "vair_agent_test_only", json!([])).await;
    let caller = f.account("caller").await;
    let caller_relay = f
        .relay(caller, agent_key, 100, json!({"team-chat": "agent-chat"}))
        .await;
    let caller_key = f.key(caller, "vair_caller_test_only", json!([])).await;

    assert_eq!(f.chat("vair_caller_test_only", "team-chat").await, 200);
    assert_eq!(
        f.last_seen(),
        Some(("Bearer owner-upstream-test-only".into(), "gpt-a".into()))
    );

    // One row per hop, each pointing at the one it relayed to, and each
    // account's diagnostics name only its own channel.
    #[derive(FromRow)]
    struct Event {
        id: Uuid,
        virtual_key_id: Uuid,
        channel_id: Option<Uuid>,
        model: String,
        relayed_event_id: Option<Uuid>,
        diagnosed_channel: Option<String>,
    }
    let event = |key: Uuid| {
        sqlx::query_as::<_, Event>(
            "SELECT id,virtual_key_id,channel_id,model,relayed_event_id,
                    request_diagnostics->'attempts'->0->>'channel_id' AS diagnosed_channel
             FROM usage_events WHERE virtual_key_id=$1",
        )
        .bind(key)
        .fetch_one(f.db())
    };
    let served = event(owner_key).await.unwrap();
    let middle = event(agent_key).await.unwrap();
    let outer = event(caller_key).await.unwrap();
    assert_eq!(
        (
            served.channel_id,
            served.model.as_str(),
            served.relayed_event_id
        ),
        (Some(upstream), "gpt-a", None)
    );
    assert_eq!(
        (
            middle.channel_id,
            middle.model.as_str(),
            middle.relayed_event_id
        ),
        (Some(agent_relay), "agent-chat", Some(served.id))
    );
    assert_eq!(
        (
            outer.channel_id,
            outer.model.as_str(),
            outer.relayed_event_id
        ),
        (Some(caller_relay), "team-chat", Some(middle.id))
    );
    assert_eq!(served.diagnosed_channel, Some(upstream.to_string()));
    assert_eq!(middle.diagnosed_channel, Some(agent_relay.to_string()));
    assert_eq!(outer.diagnosed_channel, Some(caller_relay.to_string()));
    assert_eq!(
        [
            served.virtual_key_id,
            middle.virtual_key_id,
            outer.virtual_key_id
        ],
        [owner_key, agent_key, caller_key]
    );

    // Model lists follow the chain too.
    let (status, catalog) = f
        .call("GET", "/v1/models", "vair_caller_test_only", None)
        .await;
    assert_eq!(status, 200);
    let ids: Vec<&str> = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["agent-chat", "gpt-a", "team-chat"]);
    assert_eq!(
        gateway::source_key_model_names(&f.state, agent_key, caller)
            .await
            .unwrap(),
        ["agent-chat", "gpt-a"]
    );
    let by_protocol = gateway::key_models_by_protocol(&f.state, caller_key)
        .await
        .unwrap();
    assert_eq!(
        by_protocol["openai"].iter().collect::<Vec<_>>(),
        ["agent-chat", "gpt-a", "team-chat"]
    );

    // A probe of the outer channel records monitor usage on every key on the path.
    let target = gateway::relay_probe_target(&f.state, caller, caller_relay, "team-chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.model, "gpt-a");
    target.record(&f.state, 200, (Some(1), 2), None).await;
    drop(target);
    let monitored: Vec<(Uuid, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT virtual_key_id,channel_id,relayed_event_id FROM usage_events
         WHERE source='monitor' ORDER BY created_at,relayed_event_id NULLS FIRST",
    )
    .fetch_all(f.db())
    .await
    .unwrap();
    assert_eq!(monitored.len(), 2);
    assert_eq!(
        (monitored[0].0, monitored[0].1, monitored[0].2),
        (owner_key, Some(upstream), None)
    );
    assert_eq!(
        (monitored[1].0, monitored[1].1),
        (agent_key, Some(agent_relay))
    );
    assert!(monitored[1].2.is_some());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relay_chains_stop_after_five_hops(db: PgPool) {
    let f = Fixture::new(db).await;
    // accounts[i] relays through the key of accounts[i + 1].
    let mut accounts = Vec::new();
    for index in 0..7 {
        accounts.push(f.account(&format!("hop {index}")).await);
    }
    f.upstream(accounts[5], "gpt-a", "five-hops-test-only", 100)
        .await;
    f.upstream(accounts[6], "gpt-b", "six-hops-test-only", 100)
        .await;
    for index in (0..6).rev() {
        let key = f
            .key(
                accounts[index + 1],
                &format!("vair_hop{}_test_only", index + 1),
                json!([]),
            )
            .await;
        f.relay(accounts[index], key, 100, json!({})).await;
    }
    f.key(accounts[0], "vair_hop0_test_only", json!([])).await;

    assert_eq!(f.chat("vair_hop0_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer five-hops-test-only");
    f.seen.lock().unwrap().clear();
    assert_eq!(f.chat("vair_hop0_test_only", "gpt-b").await, 404);
    assert!(f.last_seen().is_none());
    // Five hops along, every account in the chain has a row.
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM usage_events WHERE model='gpt-a' AND status=200")
            .fetch_one(f.db())
            .await
            .unwrap();
    assert_eq!(rows, 6);

    let (_, catalog) = f
        .call("GET", "/v1/models", "vair_hop0_test_only", None)
        .await;
    let ids: Vec<&str> = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["gpt-a"]);
    // The sixth account is one hop from the first relay's own account.
    assert_eq!(f.chat("vair_hop1_test_only", "gpt-b").await, 200);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn relay_cycles_through_several_accounts_end(db: PgPool) {
    let f = Fixture::new(db).await;
    let first = f.account("first").await;
    let second = f.account("second").await;
    let third = f.account("third").await;
    let first_key = f.key(first, "vair_first_test_only", json!([])).await;
    let second_key = f.key(second, "vair_second_test_only", json!([])).await;
    let third_key = f.key(third, "vair_third_test_only", json!([])).await;
    f.relay(first, second_key, 100, json!({})).await;
    f.relay(second, third_key, 100, json!({})).await;
    f.relay(third, first_key, 100, json!({})).await;

    assert_eq!(f.chat("vair_first_test_only", "gpt-a").await, 404);
    assert!(f.last_seen().is_none());
    let (status, catalog) = f
        .call("GET", "/v1/models", "vair_first_test_only", None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(catalog["data"], json!([]));

    // An upstream anywhere on the cycle is reached from every account.
    f.upstream(third, "gpt-a", "third-upstream-test-only", 100)
        .await;
    assert_eq!(f.chat("vair_first_test_only", "gpt-a").await, 200);
    assert_eq!(f.chat("vair_second_test_only", "gpt-a").await, 200);
    assert_eq!(f.chat("vair_third_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer third-upstream-test-only");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn inner_source_keys_restrict_relayed_requests(db: PgPool) {
    let f = Fixture::new(db).await;
    let owner = f.account("owner").await;
    f.upstream(owner, "gpt-a", "owner-upstream-test-only", 100)
        .await;
    let owner_key = f.key(owner, "vair_owner_test_only", json!([])).await;
    let agent = f.account("agent").await;
    f.relay(agent, owner_key, 100, json!({})).await;
    let agent_key = f.key(agent, "vair_agent_test_only", json!([])).await;
    let caller = f.account("caller").await;
    f.relay(caller, agent_key, 10, json!({})).await;
    f.upstream(caller, "gpt-a", "caller-upstream-test-only", 200)
        .await;
    f.key(caller, "vair_caller_test_only", json!([])).await;

    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer owner-upstream-test-only");

    // The innermost key's model permissions apply two hops away; the caller
    // fails over to its own upstream.
    sqlx::query("UPDATE virtual_api_keys SET allowed_models='[\"other\"]' WHERE id=$1")
        .bind(owner_key)
        .execute(f.db())
        .await
        .unwrap();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer caller-upstream-test-only");
    assert!(
        gateway::source_key_model_names(&f.state, agent_key, caller)
            .await
            .unwrap()
            .is_empty()
    );

    // So does a paused key in the middle.
    sqlx::query("UPDATE virtual_api_keys SET allowed_models='[]' WHERE id=$1")
        .bind(owner_key)
        .execute(f.db())
        .await
        .unwrap();
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(agent_key)
        .execute(f.db())
        .await
        .unwrap();
    f.seen.lock().unwrap().clear();
    assert_eq!(f.chat("vair_caller_test_only", "gpt-a").await, 200);
    assert_eq!(f.last_seen().unwrap().0, "Bearer caller-upstream-test-only");
}
