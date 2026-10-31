use super::*;
use crate::{
    crypto::Crypto,
    quality_monitors::{self as qm, CreateInput, Settings},
};
use axum::{
    Json, Router,
    response::{IntoResponse, Response},
    routing::post,
};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, Notify};

/// A mock Anthropic answer to one service-check or long-context request. A
/// dishonest upstream also caps output, breaks streams and drops long inputs.
fn service_check_answer(prompt: &str, body: &Value, honest: bool, reports_usage: bool) -> Response {
    let input = prompt.chars().count() as i64 / 4 + 10 + if honest { 0 } else { 2000 };
    let usage = if reports_usage {
        json!({"input_tokens":input,"output_tokens":5})
    } else {
        Value::Null
    };
    let code = prompt
        .split(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .find(|w| w.split('-').filter(|part| !part.is_empty()).count() == 4);
    if body["stream"] == true {
        let text = if honest { code.unwrap() } else { "WRONG" };
        let delta = json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}});
        let end = json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":usage});
        let mut sse = format!(
            "event: content_block_delta\ndata: {delta}\n\nevent: message_delta\ndata: {end}\n\n"
        );
        if honest {
            sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
        }
        return (
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            sse,
        )
            .into_response();
    }
    if let Some(rest) = prompt.strip_prefix("请输出 ") {
        let lines: usize = rest.split(' ').next().unwrap().parse().unwrap();
        let marker = prompt
            .split("k. ")
            .nth(1)
            .unwrap()
            .split('」')
            .next()
            .unwrap();
        let shown = if honest { lines } else { 10 };
        let text: String = (1..=shown).map(|k| format!("{k}. {marker}\n")).collect();
        return Json(
            json!({"content":[{"type":"text","text":text}],"usage":usage,
            "stop_reason":if honest { "end_turn" } else { "max_tokens" }}),
        )
        .into_response();
    }
    if let Some(at) = prompt.find("The secret passphrase is ") {
        let passphrase = prompt[at + 25..].split('.').next().unwrap();
        let text = if honest {
            passphrase
        } else {
            "没有找到口令"
        };
        return Json(
            json!({"content":[{"type":"text","text":text}],"usage":usage,"stop_reason":"end_turn"}),
        )
        .into_response();
    }
    if body.get("tools").is_some() {
        let order = prompt
            .split_whitespace()
            .flat_map(|w| w.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')))
            .find(|w| w.starts_with("ORD-"))
            .unwrap();
        return if honest {
            Json(json!({"content":[{"type":"tool_use","name":"lookup_order","input":{"order_id":order}}],"usage":usage})).into_response()
        } else {
            Json(json!({"content":[{"type":"text","text":"订单已发货"}],"usage":usage}))
                .into_response()
        };
    }
    let text = match code {
        Some(code) if honest => {
            let ints = crate::authenticity::fingerprint::parse_ints(prompt);
            format!("{code}\n{}", ints[ints.len() - 2] + ints[ints.len() - 1])
        }
        Some(_) => "WRONG".into(),
        None => "OK".into(),
    };
    Json(json!({"content":[{"type":"text","text":text}],"usage":usage,"stop_reason":"end_turn"}))
        .into_response()
}

struct Fixture {
    state: AppState,
    account: Uuid,
    channel: Uuid,
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
        let pause = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let mock = Router::new().route(
            "/v1/messages",
            post({
                let pause = pause.clone();
                let entered = entered.clone();
                let release = release.clone();
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let pause = pause.clone();
                    let entered = entered.clone();
                    let release = release.clone();
                    async move {
                        assert_eq!(headers["x-api-key"], "quality-test-only-credential");
                        if pause.swap(false, Ordering::SeqCst) {
                            entered.notify_one();
                            release.notified().await;
                        }
                        let model = body["model"].as_str().unwrap();
                        assert!(matches!(
                            model,
                            "good-upstream" | "bad-upstream" | "quiet-upstream" | "blank-upstream"
                        ));
                        let prompt = body["messages"][0]["content"].as_str().unwrap();
                        // good-upstream answers honestly; bad-upstream tampers with
                        // answers, hides a system prompt and ignores tools;
                        // quiet-upstream is honest without usage; blank-upstream
                        // returns empty answers.
                        if model == "blank-upstream" {
                            return Json(
                                json!({"content":[],"usage":{"input_tokens":10,"output_tokens":0},
                                "stop_reason":"end_turn"}),
                            )
                            .into_response();
                        }
                        service_check_answer(
                            prompt,
                            &body,
                            model != "bad-upstream",
                            model != "quiet-upstream",
                        )
                    }
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let crypto = Arc::new(Crypto::from_env().unwrap());
        let state = AppState {
            db: db.clone(),
            redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
            http: reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(proxy_url).unwrap())
                .build()
                .unwrap(),
            crypto: crypto.clone(),
            semaphores: Arc::new(Mutex::new(HashMap::new())),
            cookie_secure: false,
            super_admin_emails: Arc::default(),
            redis_connection: Default::default(),
            routing_settings: Default::default(),
            prices: Default::default(),
        };
        let account =
            sqlx::query_scalar("INSERT INTO accounts(name) VALUES('quality test') RETURNING id")
                .fetch_one(&db)
                .await
                .unwrap();
        let channel=sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,models,model_mapping)
            VALUES($1,'quality fixture','anthropic','http://1.1.1.1','[\"good\",\"bad\"]','{\"good\":\"good-upstream\",\"bad\":\"bad-upstream\"}') RETURNING id")
            .bind(account).fetch_one(&db).await.unwrap();
        sqlx::query(
            "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES($1,'fixture',$2)",
        )
        .bind(channel)
        .bind(crypto.encrypt("quality-test-only-credential").unwrap())
        .execute(&db)
        .await
        .unwrap();
        Self {
            state,
            account,
            channel,
            pause,
            entered,
            release,
            server,
        }
    }
    async fn add(&self, model: &str, settings: Settings) -> Uuid {
        qm::insert(
            &self.state.db,
            self.account,
            &CreateInput {
                channel_id: self.channel,
                model: model.into(),
                settings,
                run_immediately: Some(false),
                kind: "service".into(),
                authenticity: None,
            },
        )
        .await
        .unwrap()
        .0
    }
    async fn run(&self, id: Uuid) {
        let job = qm::enqueue(&self.state.db, self.account, id, "manual")
            .await
            .unwrap();
        assert_eq!(claim_job(&self.state).await.unwrap(), Some(job));
        execute_job(&self.state, job).await.unwrap();
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn manual_monitor_crud_isolation_and_budget(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let settings = Settings {
        daily_run_limit: 1,
        ..Default::default()
    };
    let input = CreateInput {
        channel_id: f.channel,
        model: "good".into(),
        settings: settings.clone(),
        run_immediately: Some(true),
        kind: "service".into(),
        authenticity: None,
    };
    assert!(matches!(
        qm::insert(&db, Uuid::new_v4(), &input).await,
        Err(AppError::NotFound)
    ));
    let (id, job) = qm::insert(&db, f.account, &input).await.unwrap();
    assert!(matches!(
        qm::insert(&db, f.account, &input).await,
        Err(AppError::Conflict(_))
    ));
    assert!(qm::load(&db, Uuid::new_v4()).await.unwrap().is_empty());
    assert!(matches!(
        qm::enqueue(&db, Uuid::new_v4(), id, "manual").await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        qm::change_settings(&db, Uuid::new_v4(), id, &settings).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        qm::remove(&db, Uuid::new_v4(), id).await,
        Err(AppError::NotFound)
    ));
    // Duplicate clicks return the same job even when the budget has been reserved.
    assert_eq!(
        Some(qm::enqueue(&db, f.account, id, "manual").await.unwrap()),
        job
    );
    assert_eq!(claim_job(&f.state).await.unwrap(), job);
    execute_job(&f.state, job.unwrap()).await.unwrap();
    let monitors = qm::load(&db, f.account).await.unwrap();
    assert_eq!(monitors[0].status, "healthy");
    assert_eq!(monitors[0].score, Some(100.0));
    let checks: Vec<String> = monitors[0]
        .last_tests
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        checks,
        [
            "integrity",
            "replay",
            "metering",
            "tools",
            "truncation",
            "streaming"
        ]
    );
    assert_eq!(monitors[0].upstream_model, "good-upstream");
    assert!(monitors[0].next_run_at.is_none());
    assert!(!monitors[0].schedule_enabled);
    assert!(matches!(
        qm::enqueue(&db, f.account, id, "manual").await,
        Err(AppError::Conflict(_))
    ));
    schedule_due(&f.state).await.unwrap();
    assert!(claim_job(&f.state).await.unwrap().is_none());
    qm::remove(&db, f.account, id).await.unwrap();
    assert!(qm::load(&db, f.account).await.unwrap().is_empty());
    qm::ensure_owned(&db, f.account, id, true).await.unwrap();
    assert!(
        qm::ensure_owned(&db, Uuid::new_v4(), id, true)
            .await
            .is_err()
    );
    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM channel_monitor_runs WHERE monitor_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(history, 1);
    assert!(f.add("good", Settings::default()).await != id);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn model_results_and_routing_are_independent(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let mut settings = Settings {
        route_policy: "exclude".into(),
        ..Default::default()
    };
    let good = f.add("good", settings.clone()).await;
    let bad = f.add("bad", settings.clone()).await;
    // Cold start must not remove an untested model from routing.
    assert_eq!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "good")
            .await
            .unwrap(),
        vec![f.channel]
    );
    f.run(good).await;
    f.run(bad).await;
    assert!(
        !gateway::quality_test_candidates(&f.state, f.account, "anthropic", "bad")
            .await
            .unwrap()
            .is_empty()
    );
    f.run(bad).await;
    assert!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "bad")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "good")
            .await
            .unwrap(),
        vec![f.channel]
    );
    // Route aliases share the same resolver as gateway traffic and invalidate
    // their verdict when the upstream target changes through the route API.
    let route: Uuid = sqlx::query_scalar(
        "INSERT INTO model_routes(account_id,channel_id,protocol,public_model,upstream_model)
        VALUES($1,$2,'anthropic','alias','good-upstream') RETURNING id",
    )
    .bind(f.account)
    .bind(f.channel)
    .fetch_one(&db)
    .await
    .unwrap();
    let alias = f.add("alias", settings.clone()).await;
    f.run(alias).await;
    assert_eq!(
        qm::load(&db, f.account)
            .await
            .unwrap()
            .iter()
            .find(|m| m.id == alias)
            .unwrap()
            .status,
        "healthy"
    );
    sqlx::query("UPDATE model_routes SET upstream_model='bad-upstream' WHERE id=$1")
        .bind(route)
        .execute(&db)
        .await
        .unwrap();
    let aliases = qm::load(&db, f.account).await.unwrap();
    let changed = aliases.iter().find(|m| m.id == alias).unwrap();
    assert_eq!(changed.status, "unknown");
    assert!(changed.score.is_none());
    assert_eq!(changed.upstream_model, "bad-upstream");
    let results = qm::load(&db, f.account).await.unwrap();
    assert_eq!(
        results.iter().find(|m| m.id == good).unwrap().score,
        Some(100.0)
    );
    assert_eq!(
        results.iter().find(|m| m.id == bad).unwrap().score,
        Some(0.0)
    );
    settings.route_policy = "observe".into();
    qm::change_settings(&db, f.account, bad, &settings)
        .await
        .unwrap();
    assert_eq!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "bad")
            .await
            .unwrap(),
        vec![f.channel]
    );
    let backup: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,models,priority)
        VALUES($1,'backup','anthropic','http://1.1.1.1','[\"bad\"]',200) RETURNING id",
    )
    .bind(f.account)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channel_keys(channel_id,label,secret_ciphertext) SELECT $1,'fixture',secret_ciphertext FROM channel_keys WHERE channel_id=$2")
        .bind(backup).bind(f.channel).execute(&db).await.unwrap();
    settings.route_policy = "deprioritize".into();
    qm::change_settings(&db, f.account, bad, &settings)
        .await
        .unwrap();
    assert_eq!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "bad")
            .await
            .unwrap(),
        vec![backup, f.channel]
    );
    settings.route_policy = "exclude".into();
    qm::change_settings(&db, f.account, bad, &settings)
        .await
        .unwrap();
    sqlx::query("UPDATE quality_monitors SET valid_until=now()-interval '1 second' WHERE id=$1")
        .bind(bad)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        gateway::quality_test_candidates(&f.state, f.account, "anthropic", "bad")
            .await
            .unwrap(),
        vec![f.channel, backup]
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn scheduling_pause_resume_and_job_deduplication(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let mut settings = Settings {
        schedule_enabled: true,
        ..Default::default()
    };
    let id = f.add("good", settings.clone()).await;
    schedule_due(&f.state).await.unwrap();
    schedule_due(&f.state).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_monitor_jobs WHERE monitor_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 1);
    settings.schedule_enabled = false;
    qm::change_settings(&db, f.account, id, &settings)
        .await
        .unwrap();
    assert!(claim_job(&f.state).await.unwrap().is_none());
    f.run(id).await;
    let partial = qm::patch_settings(&db, f.account, id, &json!({"interval_seconds":900}))
        .await
        .unwrap();
    assert!(!partial.schedule_enabled);
    assert_eq!(partial.timeout_ms, settings.timeout_ms);
    assert_eq!(partial.methods, settings.methods);
    assert!(
        qm::patch_settings(&db, f.account, id, &json!({"model":"bad"}))
            .await
            .is_err()
    );
    settings.schedule_enabled = true;
    qm::change_settings(&db, f.account, id, &settings)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(f.channel)
        .execute(&db)
        .await
        .unwrap();
    schedule_due(&f.state).await.unwrap();
    assert!(claim_job(&f.state).await.unwrap().is_none());
    assert!(qm::enqueue(&db, f.account, id, "manual").await.is_err());
    sqlx::query("UPDATE channels SET enabled=true WHERE id=$1")
        .bind(f.channel)
        .execute(&db)
        .await
        .unwrap();
    let monitors = qm::load(&db, f.account).await.unwrap();
    assert!(monitors[0].schedule_enabled);
    assert_eq!(monitors[0].status, "unknown");
    schedule_due(&f.state).await.unwrap();
    let (a, b) = tokio::join!(claim_job(&f.state), claim_job(&f.state));
    assert_eq!(
        usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn editing_or_deleting_during_execution_cannot_apply_stale_results(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    for deleting in [false, true] {
        let id = f
            .add(if deleting { "bad" } else { "good" }, Settings::default())
            .await;
        let job = qm::enqueue(&db, f.account, id, "manual").await.unwrap();
        assert_eq!(claim_job(&f.state).await.unwrap(), Some(job));
        f.pause.store(true, Ordering::SeqCst);
        let state = f.state.clone();
        let handle = tokio::spawn(async move { execute_job(&state, job).await });
        tokio::time::timeout(std::time::Duration::from_secs(5), f.entered.notified())
            .await
            .unwrap();
        if deleting {
            qm::remove(&db, f.account, id).await.unwrap();
        } else {
            qm::change_settings(
                &db,
                f.account,
                id,
                &Settings {
                    min_score: 100.0,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        f.release.notify_one();
        handle.await.unwrap().unwrap();
        let row = sqlx::query("SELECT status,score FROM quality_monitors WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("status"), "unknown");
        assert!(row.get::<Option<f64>, _>("score").is_none());
        let applied: bool = sqlx::query_scalar(
            "SELECT (details->>'applied')::boolean FROM channel_monitor_runs WHERE monitor_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(!applied);
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn signed_in_users_see_their_account_history(db: PgPool) {
    use axum::{
        extract::{Query, State},
        http::HeaderMap,
    };
    let f = Fixture::new(db.clone()).await;
    let user:Uuid=sqlx::query_scalar("INSERT INTO users(email,password_hash) VALUES('monitor-owner@example.invalid','test-only') RETURNING id").fetch_one(&db).await.unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES($1,$2)")
        .bind(f.account)
        .bind(user)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sessions(user_id,token_hash,expires_at) VALUES($1,$2,now()+interval '1 hour')",
    )
    .bind(user)
    .bind(crate::auth::hash_token("test-session"))
    .execute(&db)
    .await
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", "gateway_session=test-session".parse().unwrap());
    let id = f.add("good", Settings::default()).await;
    assert_eq!(
        qm::list(State(f.state.clone()), headers.clone())
            .await
            .unwrap()
            .0
            .len(),
        1
    );
    f.run(id).await;
    let history = crate::monitoring_runs(
        State(f.state.clone()),
        headers.clone(),
        Query(crate::MonitorRunsQuery {
            monitor_id: Some(id),
            channel_id: None,
            kind: Some(crate::MonitorKind::Quality),
            limit: Some(10),
            page: Some(1),
            per_channel: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(history["total"], 1);
    assert_eq!(history["items"][0]["monitor_id"], id.to_string());
    let heat = crate::monitoring_runs(
        State(f.state.clone()),
        headers,
        Query(crate::MonitorRunsQuery {
            monitor_id: None,
            channel_id: None,
            kind: Some(crate::MonitorKind::Quality),
            limit: None,
            page: None,
            per_channel: Some(12),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(heat.as_array().unwrap().len(), 1);
}

/// Monitor rows as a migration left them. Migration tests read the columns
/// directly because `qm::load` follows the latest schema.
#[derive(sqlx::FromRow)]
struct MigratedMonitor {
    model: String,
    schedule_enabled: bool,
    min_score: f64,
    status: String,
    score: Option<f64>,
    route_policy: String,
}

async fn migrated_monitors(db: &PgPool, account: Uuid) -> Vec<MigratedMonitor> {
    sqlx::query_as(
        "SELECT q.model,q.schedule_enabled,q.min_score,q.status,q.score,q.route_policy
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE c.account_id=$1 AND q.deleted_at IS NULL ORDER BY q.model",
    )
    .bind(account)
    .fetch_all(db)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn migration_splits_models_and_preserves_legacy_history(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 32) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('migration fixture') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let channel:Uuid=sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,test_model) VALUES($1,'legacy','anthropic','http://1.1.1.1','fallback') RETURNING id")
        .bind(account).fetch_one(&db).await.unwrap();
    sqlx::query("INSERT INTO channel_monitor_configs(channel_id,quality_enabled,quality_models,quality_min_score) VALUES($1,true,'[\"a\",\"b\",\"a\"]',82)").bind(channel).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO channel_health(channel_id,status,quality_status,quality_score) VALUES($1,'healthy','healthy',100)").bind(channel).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO channel_monitor_runs(channel_id,kind,model,status,score,started_at,finished_at) VALUES($1,'quality','a, b','healthy',100,now(),now())").bind(channel).execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0032_model_quality_monitors.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let rows = migrated_monitors(&db, account).await;
    assert_eq!(rows.len(), 2);
    for m in rows {
        assert!(m.schedule_enabled);
        assert_eq!(m.min_score, 82.0);
        assert_eq!(m.status, "unknown");
        assert!(m.score.is_none());
        assert_eq!(m.route_policy, "exclude");
    }
    let legacy: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_monitor_runs WHERE monitor_id IS NULL AND score=100",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(legacy, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn migration_replaces_prefix_monitors_with_a_concrete_model(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 35) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('migration fixture') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let channel:Uuid=sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES($1,'legacy','openai','http://1.1.1.1','[\"demo-*\",\"demo-chat\"]') RETURNING id")
        .bind(account).fetch_one(&db).await.unwrap();
    // 0032 copied the first legacy channel rule, which may be a prefix.
    let prefix:Uuid=sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,schedule_enabled,route_policy,min_score,next_run_at) VALUES($1,'demo-*',true,'exclude',82,now()) RETURNING id")
        .bind(channel).fetch_one(&db).await.unwrap();
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO quality_monitor_jobs(monitor_id,config_version,source) VALUES($1,1,'scheduled') RETURNING id",
    )
    .bind(prefix)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0035_model_management_corrections.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let rows = migrated_monitors(&db, account).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model, "demo-chat");
    assert!(rows[0].schedule_enabled);
    assert_eq!(rows[0].route_policy, "exclude");
    assert_eq!(rows[0].min_score, 82.0);
    let status: String = sqlx::query_scalar("SELECT status FROM quality_monitor_jobs WHERE id=$1")
        .bind(job)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
    assert!(
        sqlx::query("INSERT INTO quality_monitors(channel_id,model) VALUES($1,'demo-next-*')")
            .bind(channel)
            .execute(&db)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn unfinished_side_checks_still_conclude_and_blank_answers_fail(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    sqlx::query(
        "UPDATE channels SET models='[\"quiet\",\"blank\"]',
        model_mapping='{\"quiet\":\"quiet-upstream\",\"blank\":\"blank-upstream\"}' WHERE id=$1",
    )
    .bind(f.channel)
    .execute(&db)
    .await
    .unwrap();
    let quiet = f.add("quiet", Settings::default()).await;
    let blank = f.add("blank", Settings::default()).await;
    f.run(quiet).await;
    f.run(blank).await;
    let monitors = qm::load(&db, f.account).await.unwrap();
    // Without usage the metering check cannot finish, yet the round concludes
    // over the five checks that did and its result stays valid.
    let quiet = monitors.iter().find(|m| m.id == quiet).unwrap();
    assert_eq!(quiet.last_observation, "healthy");
    assert_eq!(quiet.score, Some(100.0));
    assert!(quiet.valid_until.is_some());
    let tests = quiet.last_tests.as_ref().unwrap().as_array().unwrap();
    let metering = tests.iter().find(|t| t["id"] == "metering").unwrap();
    assert_eq!(metering["error_code"], "no_usage", "{metering}");
    // A blank answer is a failed integrity check, not an unfinished one.
    let blank = monitors.iter().find(|m| m.id == blank).unwrap();
    assert_eq!(blank.last_observation, "degraded");
    let tests = blank.last_tests.as_ref().unwrap().as_array().unwrap();
    let integrity = tests.iter().find(|t| t["id"] == "integrity").unwrap();
    assert_eq!(integrity["passed"], false);
    assert!(integrity["error_code"].is_null());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn stopped_channels_save_monitors_and_intervals_rescale_validity(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(f.channel)
        .execute(&db)
        .await
        .unwrap();
    let input = |run_immediately| CreateInput {
        channel_id: f.channel,
        model: "good".into(),
        settings: Settings::default(),
        run_immediately,
        kind: "service".into(),
        authenticity: None,
    };
    assert!(matches!(
        qm::insert(&db, f.account, &input(Some(true))).await,
        Err(AppError::Conflict(_))
    ));
    let (id, job) = qm::insert(&db, f.account, &input(None)).await.unwrap();
    assert!(job.is_none());
    // A verdict from a 1h interval is valid for 2h; at 15 minutes it lasts 30 minutes.
    sqlx::query("UPDATE quality_monitors SET status='degraded',checked_at=now(),valid_until=now()+interval '2 hours' WHERE id=$1")
        .bind(id).execute(&db).await.unwrap();
    qm::change_settings(
        &db,
        f.account,
        id,
        &Settings {
            interval_seconds: 900,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, remaining): (String, f64) = sqlx::query_as(
        "SELECT status,EXTRACT(EPOCH FROM valid_until-now())::float8 FROM quality_monitors WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(status, "degraded");
    assert!((1700.0..=1800.0).contains(&remaining), "{remaining}");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn service_check_migration_replaces_fixed_questions(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 50) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('migration fixture') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let channel: Uuid = sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES($1,'legacy','openai','http://1.1.1.1','[\"m\"]') RETURNING id")
        .bind(account).fetch_one(&db).await.unwrap();
    let monitor: Uuid = sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,methods,status,score,schedule_enabled,route_policy) VALUES($1,'m','[\"arithmetic\",\"json\"]','degraded',50,true,'exclude') RETURNING id")
        .bind(channel).fetch_one(&db).await.unwrap();
    let job: Uuid = sqlx::query_scalar("INSERT INTO quality_monitor_jobs(monitor_id,config_version,source) VALUES($1,1,'scheduled') RETURNING id")
        .bind(monitor).fetch_one(&db).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/0050_service_checks.sql"))
        .execute(&db)
        .await
        .unwrap();
    let (methods, status, score, route_policy): (Value, String, Option<f64>, String) =
        sqlx::query_as(
            "SELECT methods,status,score,route_policy FROM quality_monitors WHERE id=$1",
        )
        .bind(monitor)
        .fetch_one(&db)
        .await
        .unwrap();
    // Old results no longer apply to routing; settings other than methods are kept.
    assert_eq!(methods, json!(["integrity", "metering", "tools"]));
    assert_eq!((status.as_str(), score), ("unknown", None));
    assert_eq!(route_policy, "exclude");
    let status: String = sqlx::query_scalar("SELECT status FROM quality_monitor_jobs WHERE id=$1")
        .bind(job)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
}

// Authenticity checks are a hosted-service feature.
#[cfg(feature = "cloud")]
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn service_and_authenticity_monitors_are_separate(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let service = f.add("good", Settings::default()).await;
    let input = CreateInput {
        channel_id: f.channel,
        model: "good".into(),
        settings: Settings {
            schedule_enabled: true,
            route_policy: "exclude".into(),
            ..Default::default()
        },
        run_immediately: Some(true),
        kind: "authenticity".into(),
        authenticity: None,
    };
    // One of each kind per channel and model; settings never apply to authenticity monitors.
    let (authenticity, job) = qm::insert(&db, f.account, &input).await.unwrap();
    assert!(job.is_none());
    assert!(matches!(
        qm::insert(&db, f.account, &input).await,
        Err(AppError::Conflict(_))
    ));
    let rows = qm::load(&db, f.account).await.unwrap();
    let row = rows.iter().find(|m| m.id == authenticity).unwrap();
    assert_eq!(
        (
            row.kind.as_str(),
            row.schedule_enabled,
            row.route_policy.as_str()
        ),
        ("authenticity", false, "observe")
    );
    assert!(matches!(
        qm::enqueue(&db, f.account, authenticity, "manual").await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        qm::change_settings(&db, f.account, authenticity, &Settings::default()).await,
        Err(AppError::BadRequest(_))
    ));
    // Deleting one leaves the other.
    qm::remove(&db, f.account, authenticity).await.unwrap();
    assert_eq!(
        qm::load(&db, f.account)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![service]
    );
    assert!(matches!(
        qm::insert(
            &db,
            f.account,
            &CreateInput {
                kind: "other".into(),
                authenticity: None,
                ..input
            }
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn migration_splits_authenticity_history_into_its_own_monitor(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 51) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('migration fixture') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let channel: Uuid = sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES($1,'relay','openai','http://1.1.1.1','[\"m\",\"n\"]') RETURNING id")
        .bind(account).fetch_one(&db).await.unwrap();
    let combined: Uuid = sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,status,route_policy,identity_verdict,identity_checked_at) VALUES($1,'m','degraded','exclude','match',now()) RETURNING id")
        .bind(channel).fetch_one(&db).await.unwrap();
    let plain: Uuid = sqlx::query_scalar(
        "INSERT INTO quality_monitors(channel_id,model) VALUES($1,'n') RETURNING id",
    )
    .bind(channel)
    .fetch_one(&db)
    .await
    .unwrap();
    let deep: Uuid = sqlx::query_scalar("INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,status,finished_at) VALUES($1,1,'deep','completed',now()) RETURNING id")
        .bind(combined).fetch_one(&db).await.unwrap();
    let service_job: Uuid = sqlx::query_scalar("INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,status,finished_at) VALUES($1,1,'manual','completed',now()) RETURNING id")
        .bind(combined).fetch_one(&db).await.unwrap();
    sqlx::query("INSERT INTO authenticity_samples(job_id,monitor_id,probe_kind,template_id,valid) VALUES($1,$2,'fingerprint',gen_random_uuid(),true)")
        .bind(deep).bind(combined).execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0051_split_authenticity_monitors.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let split: (Uuid, Option<String>) = sqlx::query_as(
        "SELECT id,identity_verdict FROM quality_monitors WHERE channel_id=$1 AND model='m' AND kind='authenticity'",
    )
    .bind(channel)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(split.1.as_deref(), Some("match"));
    let owner = |table: &'static str, id: Uuid| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(&format!("SELECT monitor_id FROM {table} WHERE id=$1"))
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(owner("quality_monitor_jobs", deep).await, split.0);
    assert_eq!(owner("quality_monitor_jobs", service_job).await, combined);
    let sample_owner: Uuid =
        sqlx::query_scalar("SELECT monitor_id FROM authenticity_samples WHERE job_id=$1")
            .bind(deep)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(sample_owner, split.0);
    // The service monitor keeps its routing state and loses the identity fields.
    let (kind, status, verdict): (String, String, Option<String>) =
        sqlx::query_as("SELECT kind,status,identity_verdict FROM quality_monitors WHERE id=$1")
            .bind(combined)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (kind.as_str(), status.as_str(), verdict),
        ("service", "degraded", None)
    );
    // Monitors without authenticity history are not split.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM quality_monitors WHERE channel_id=$1 AND model='n'",
    )
    .bind(channel)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let _ = plain;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn long_context_checks_record_the_largest_tier_without_touching_status(db: PgPool) {
    use crate::long_context::{self, StartInput};
    let f = Fixture::new(db.clone()).await;
    let good = f.add("good", Settings::default()).await;
    let bad = f.add("bad", Settings::default()).await;
    let input = |acknowledge_risk| StartInput {
        max_tokens: 32_000,
        acknowledge_risk,
    };
    let user = Uuid::new_v4();
    assert!(matches!(
        long_context::enqueue(&db, f.account, user, good, &input(false)).await,
        Err(AppError::Refused("authenticity_ack_required", _))
    ));
    assert!(matches!(
        long_context::enqueue(
            &db,
            f.account,
            user,
            good,
            &StartInput {
                max_tokens: 1000,
                acknowledge_risk: true
            }
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    // Acknowledged once per channel, as for authenticity checks.
    sqlx::query("UPDATE channels SET authenticity_ack_at=now() WHERE id=$1")
        .bind(f.channel)
        .execute(&db)
        .await
        .unwrap();
    for id in [good, bad] {
        let job = long_context::enqueue(&db, f.account, user, id, &input(false))
            .await
            .unwrap();
        // One job per monitor: a service round waits for the long-context check.
        assert_eq!(
            qm::enqueue(&db, f.account, id, "manual").await.unwrap(),
            job
        );
        assert_eq!(claim_job(&f.state).await.unwrap(), Some(job));
        execute_job(&f.state, job).await.unwrap();
    }
    let monitors = qm::load(&db, f.account).await.unwrap();
    let by = |id: Uuid| monitors.iter().find(|m| m.id == id).unwrap();
    assert_eq!(by(good).long_context_max, Some(32_000));
    // The upstream that drops long inputs fails the first tier and stops.
    assert_eq!(by(bad).long_context_max, None);
    assert!(by(bad).long_context_checked_at.is_some());
    // Long-context results never change the service status.
    assert_eq!(by(bad).status, "unknown");
    let runs: Vec<(String, Value)> = sqlx::query_as(
        "SELECT status,details FROM channel_monitor_runs WHERE monitor_id=$1 AND kind='long_context'",
    )
    .bind(bad)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, "degraded");
    assert_eq!(runs[0].1["tiers"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn passive_metering_waits_for_service_checks_to_confirm(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let bad = f.add("bad", Settings::default()).await;
    let good = f.add("good", Settings::default()).await;
    // A day of real traffic: the bad upstream adds 3000 tokens to every request.
    for i in 0..60_i64 {
        let ascii = 500 + i * 400;
        for (model, extra) in [("good-upstream", 0), ("bad-upstream", 3000)] {
            sqlx::query(
                "INSERT INTO usage_events(account_id,channel_id,model,upstream_model,protocol,status,duration_ms,
                   input_tokens,output_tokens,input_ascii_chars,input_other_chars,stream,created_at)
                 VALUES($1,$2,'m',$3,'anthropic',200,1000,$4,50,$5,0,true,now()-make_interval(mins=>$6::int))",
            )
            .bind(f.account)
            .bind(f.channel)
            .bind(model)
            .bind(ascii * 3 / 10 + 80 + extra)
            .bind(ascii as i32)
            .bind(i as i32)
            .execute(&db)
            .await
            .unwrap();
        }
    }
    crate::metering::run(&f.state).await.unwrap();
    let stats: Vec<(String, Value, Value)> = sqlx::query_as(
        "SELECT upstream_model,warnings,alerted FROM channel_passive_stats WHERE account_id=$1 ORDER BY upstream_model",
    )
    .bind(f.account)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].0, "bad-upstream");
    assert_eq!(stats[0].1[0]["kind"], "hidden_prompt");
    assert_eq!(stats[0].2, json!(["hidden_prompt"]));
    assert_eq!(stats[1].1, json!([]));
    // Only the flagged model's service check runs early.
    let triggered: Vec<Uuid> =
        sqlx::query_scalar("SELECT monitor_id FROM quality_monitor_jobs WHERE source='triggered'")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(triggered, vec![bad]);
    assert_ne!(triggered, vec![good]);
    // A service check can confirm a hidden prompt, so nobody is emailed yet.
    let alerts = |db: PgPool| async move {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT upstream_model,kind,source FROM metering_alerts ORDER BY upstream_model,kind",
        )
        .fetch_all(&db)
        .await
        .unwrap()
    };
    assert_eq!(alerts(db.clone()).await, vec![]);
    // A second pass does not trigger again while the check is queued.
    crate::metering::run(&f.state).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_monitor_jobs WHERE source='triggered'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 1);
    // One estimated failure is not enough; the second confirms and alerts.
    let job = claim_job(&f.state).await.unwrap().unwrap();
    execute_job(&f.state, job).await.unwrap();
    assert_eq!(alerts(db.clone()).await, vec![]);
    f.run(bad).await;
    assert_eq!(
        alerts(db.clone()).await,
        vec![(
            "bad-upstream".into(),
            "hidden_prompt".into(),
            "check".into()
        )]
    );
    // A passing check refutes the same warning from traffic.
    f.run(good).await;
    sqlx::query("UPDATE usage_events SET input_tokens=input_tokens+3000 WHERE upstream_model='good-upstream'")
        .execute(&db)
        .await
        .unwrap();
    crate::metering::run(&f.state).await.unwrap();
    let (warnings, alerted): (Value, Value) = sqlx::query_as(
        "SELECT warnings,alerted FROM channel_passive_stats WHERE upstream_model='good-upstream'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(warnings[0]["kind"], "hidden_prompt");
    assert_eq!(warnings[0]["refuted"], true);
    assert_eq!(alerted, json!([]));
    let triggered: Vec<Uuid> =
        sqlx::query_scalar("SELECT monitor_id FROM quality_monitor_jobs WHERE source='triggered'")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(triggered, vec![bad]);
    // Without traffic in the window the result disappears.
    sqlx::query("UPDATE usage_events SET created_at=now()-interval '2 days'")
        .execute(&db)
        .await
        .unwrap();
    crate::metering::run(&f.state).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_passive_stats")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // The alert outlives hours without enough traffic.
    assert_eq!(alerts(db.clone()).await.len(), 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn migration_names_the_closest_model_the_verdict_weighed(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 54) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('migration fixture') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let channel: Uuid = sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES($1,'relay','anthropic','http://1.1.1.1','[\"m\"]') RETURNING id")
        .bind(account).fetch_one(&db).await.unwrap();
    let monitor: Uuid = sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,kind) VALUES($1,'m','authenticity') RETURNING id")
        .bind(channel).fetch_one(&db).await.unwrap();
    // A report like a real one: the noisier opus had the highest total
    // likelihood, while the verdict weighed fable-5.
    let comparisons =
        json!([{"model":"fable-5","z":-4.9},{"model":"opus","z":-2.9},{"model":"haiku","z":1.8}]);
    let mismatch = json!({"claimed":"fable-5-1","best_model":"opus","comparisons":comparisons});
    let genuine =
        json!({"claimed":"fable-5-1","best_model":"opus","comparisons":[{"model":"opus","z":2.1}]});
    let mut jobs = Vec::new();
    for evidence in [&mismatch, &genuine] {
        let id: Uuid = sqlx::query_scalar("INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,status,evidence,finished_at) VALUES($1,1,'deep','completed',$2,now()) RETURNING id")
            .bind(monitor).bind(evidence).fetch_one(&db).await.unwrap();
        jobs.push(id);
    }
    sqlx::query("UPDATE quality_monitors SET identity_evidence=$2 WHERE id=$1")
        .bind(monitor)
        .bind(json!({"job_id":jobs[0],"best_model":"opus"}))
        .execute(&db)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0054_closest_model_from_comparisons.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let best = |id: Uuid| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT evidence->>'best_model' FROM quality_monitor_jobs WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    assert_eq!(best(jobs[0]).await.as_deref(), Some("fable-5"));
    // No candidate favoured: the claim itself is closest.
    assert_eq!(best(jobs[1]).await.as_deref(), Some("fable-5-1"));
    let summary: Option<String> = sqlx::query_scalar(
        "SELECT identity_evidence->>'best_model' FROM quality_monitors WHERE id=$1",
    )
    .bind(monitor)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(summary.as_deref(), Some("fable-5"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn accounts_only_email_availability_alerts_by_default(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 65) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let existing: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts(name,alert_emails) VALUES('existing','{\"authenticity\":true,\"service\":true,\"metering\":true,\"availability\":false}') RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    for sql in [
        include_str!("../migrations/0065_alert_email_defaults.sql"),
        include_str!("../migrations/0068_reset_alert_emails.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&db).await.unwrap();
    }
    let created: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES('created') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    for id in [existing, created] {
        let alerts: Value = sqlx::query_scalar("SELECT alert_emails FROM accounts WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(
            alerts,
            json!({"authenticity":false,"service":false,"metering":false,"availability":true})
        );
    }
}

#[cfg(not(feature = "cloud"))]
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-quality-monitor.sh"]
async fn openvair_refuses_authenticity_monitors_and_hides_old_ones(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let service = f.add("good", Settings::default()).await;
    let input = CreateInput {
        channel_id: f.channel,
        model: "good".into(),
        settings: Settings::default(),
        run_immediately: Some(false),
        kind: "authenticity".into(),
        authenticity: None,
    };
    assert!(matches!(
        qm::insert(&db, f.account, &input).await,
        Err(AppError::BadRequest(_))
    ));
    // Monitors left over from a hosted-service database stay out of the list.
    sqlx::query(
        "INSERT INTO quality_monitors(channel_id,model,kind) VALUES ($1,'good','authenticity')",
    )
    .bind(f.channel)
    .execute(&db)
    .await
    .unwrap();
    let rows = qm::load(&db, f.account).await.unwrap();
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [service]);
}
