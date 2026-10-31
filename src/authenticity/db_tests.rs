//! Baseline import and deep-check flows against an isolated PostgreSQL and a
//! local mock upstream. Run with scripts/test-authenticity.sh.
use super::{
    baseline::{self, Decision, TargetInput},
    bundle,
    deep::{self, StartInput},
    public,
};
use crate::{
    crypto::Crypto,
    error::AppError,
    monitor,
    quality_monitors::{self as qm, CreateInput, Settings},
    state::AppState,
};
use axum::{Json, Router, extract::Path, http::HeaderMap, routing::post};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

const FINGERPRINT_TEMPLATE: &str = "Write {n} whole numbers from 1 to {r}, comma separated.";
const SHORT_TEXT: &str = "Repeat this short sentence exactly.";
const LONG_TEXT: &str =
    "Repeat this much longer sentence exactly, including every clause, comma and word in it.";

/// Favourite values of each synthetic model family.
fn favourites(model: &str) -> &'static [u16] {
    match model {
        "acme-big" => &[7, 42, 73, 137, 211, 300],
        "acme-small" => &[7, 42, 73, 150, 222, 333],
        _ => &[12, 88, 199, 250, 301, 350],
    }
}

/// Tokenizers differ by family: same text, different counts.
fn token_count(model: &str, text: &str) -> i64 {
    let chars = text.chars().count() as i64;
    if model.starts_with("acme") {
        chars / 3 + 5
    } else {
        chars / 2 + 5
    }
}

fn numbers(model: &str, n: usize, rng: &mut StdRng) -> Vec<i64> {
    let mut table: Vec<u16> = (1..=355).collect();
    for &f in favourites(model) {
        table.extend(std::iter::repeat_n(f, 6));
    }
    (0..n)
        .map(|_| i64::from(table[rng.gen_range(0..table.len())]))
        .collect()
}

// OpenVAIR builds only the tests that do not need authenticity monitors.
#[cfg_attr(not(feature = "cloud"), allow(dead_code))]
struct Fixture {
    state: AppState,
    admin: Uuid,
    account: Uuid,
    fingerprint_template: Uuid,
    token_templates: (Uuid, Uuid),
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[cfg_attr(not(feature = "cloud"), allow(dead_code))]
impl Fixture {
    async fn new(db: PgPool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        // The upstream serves `serves[model]`'s behaviour for each requested model.
        let serves: HashMap<&'static str, &'static str> =
            HashMap::from([("acme-big", "zeta-one"), ("zeta-one", "zeta-one")]);
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(11)));
        let mock = Router::new().route(
            "/v1/chat/completions",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let rng = rng.clone();
                    let serves = serves.clone();
                    async move {
                        assert_eq!(
                            headers["authorization"],
                            "Bearer authenticity-test-only-credential"
                        );
                        let requested = body["model"].as_str().unwrap();
                        let actual = serves[requested];
                        let prompt = body["messages"][0]["content"].as_str().unwrap();
                        let input = token_count(actual, prompt);
                        let text = if prompt.starts_with("Write ") {
                            let n: usize =
                                prompt.split_whitespace().nth(1).unwrap().parse().unwrap();
                            let values = numbers(actual, n, &mut *rng.lock().await);
                            values
                                .iter()
                                .map(i64::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        } else {
                            "ok".into()
                        };
                        Json(
                            json!({"choices":[{"message":{"content":text},"finish_reason":"stop"}],
                        "usage":{"prompt_tokens":input,"completion_tokens":100,
                            "completion_tokens_details":{"reasoning_tokens":0}}}),
                        )
                    }
                },
            ),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let crypto = Arc::new(Crypto::from_env().unwrap());
        let state = AppState {
            db: db.clone(),
            redis: redis::Client::open(
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into()),
            )
            .unwrap(),
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
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO users(email,password_hash,is_super_admin) VALUES('baseline-admin@example.invalid','test-only',true) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let account: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts(name) VALUES('authenticity test') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let mut ids = Vec::new();
        for (kind, text) in [
            ("fingerprint", FINGERPRINT_TEMPLATE),
            ("token", SHORT_TEXT),
            ("token", LONG_TEXT),
        ] {
            ids.push(
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO probe_templates(probe_kind,content_ciphertext) VALUES($1,$2) RETURNING id",
                )
                .bind(kind)
                .bind(crypto.encrypt(text).unwrap())
                .fetch_one(&db)
                .await
                .unwrap(),
            );
        }
        assert_eq!(baseline::publish_pool_version(&db, admin).await.unwrap(), 1);
        for model in ["acme-big", "acme-small", "zeta-one"] {
            sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES($1,$1)")
                .bind(model)
                .execute(&db)
                .await
                .unwrap();
            baseline::upsert_target(
                &db,
                model,
                &TargetInput {
                    family: model.split('-').next().unwrap().into(),
                    openrouter_model: format!("vendor/{model}"),
                    provider_only: "acme".into(),
                    target_samples: 60,
                    token_samples: 4,
                    reasoning: "off".into(),
                },
            )
            .await
            .unwrap();
        }
        Self {
            state,
            admin,
            account,
            fingerprint_template: ids[0],
            token_templates: (ids[1], ids[2]),
            server,
        }
    }

    async fn plan(&self) -> Uuid {
        let plan = baseline::create_plan_for(
            &self.state.db,
            self.admin,
            &["acme-big".into(), "acme-small".into(), "zeta-one".into()],
        )
        .await
        .unwrap();
        serde_json::from_value(plan["id"].clone()).unwrap()
    }

    /// A bundle of `count` fingerprint samples drawn as `behaves_like`, plus token samples.
    fn bundle(
        &self,
        plan: Uuid,
        model: &str,
        behaves_like: &str,
        count: usize,
        seed: u64,
    ) -> Vec<u8> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut samples: Vec<Value> = (0..count)
            .map(|i| {
                json!({"sample_id":format!("fp-{seed}-{i}"),"probe_kind":"fingerprint",
                    "template_id":self.fingerprint_template,"n":200,"values":numbers(behaves_like, 200, &mut rng),
                    "input_tokens":40,"output_tokens":700,"reasoning_tokens":0,"served_provider":"Acme",
                    "finish_reason":"stop"})
            })
            .collect();
        for (i, (template, text)) in [
            (self.token_templates.0, SHORT_TEXT),
            (self.token_templates.1, LONG_TEXT),
        ]
        .into_iter()
        .enumerate()
        {
            samples.push(json!({"sample_id":format!("tk-{seed}-{i}"),"probe_kind":"token","template_id":template,
                "n":null,"values":[],"input_tokens":token_count(behaves_like, text),"output_tokens":2,
                "reasoning_tokens":0,"served_provider":"Acme","finish_reason":"stop"}));
        }
        bundle::build(bundle::test_manifest(plan, model, 1), &samples)
    }

    async fn import(&self, bytes: &[u8]) -> Value {
        let (created, view) = baseline::import_bundle(&self.state.db, self.admin, bytes)
            .await
            .unwrap();
        assert!(created);
        view
    }

    /// Imports and activates a baseline for each model.
    async fn activate_all(&self) {
        let plan = self.plan().await;
        for (i, model) in ["acme-big", "acme-small", "zeta-one"]
            .into_iter()
            .enumerate()
        {
            let view = self
                .import(&self.bundle(plan, model, model, 80, 100 + i as u64))
                .await;
            let id = serde_json::from_value(view["id"].clone()).unwrap();
            let version = baseline::decide(&self.state.db, self.admin, id, Decision::NewVersion)
                .await
                .unwrap()
                .unwrap();
            baseline::activate_version(&self.state.db, version)
                .await
                .unwrap();
        }
    }

    async fn channel_monitor(&self, public: &str, upstream: &str) -> Uuid {
        let channel: Uuid = sqlx::query_scalar(
            "INSERT INTO channels(account_id,name,protocol,base_url,models,model_mapping)
             VALUES($1,$2,'openai','http://1.1.1.1',$3,$4) RETURNING id",
        )
        .bind(self.account)
        .bind(format!("relay {public}"))
        .bind(json!([public]))
        .bind(json!({ public: upstream }))
        .fetch_one(&self.state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES($1,'fixture',$2)",
        )
        .bind(channel)
        .bind(
            self.state
                .crypto
                .encrypt("authenticity-test-only-credential")
                .unwrap(),
        )
        .execute(&self.state.db)
        .await
        .unwrap();
        qm::insert(
            &self.state.db,
            self.account,
            &CreateInput {
                channel_id: channel,
                model: public.into(),
                settings: Settings::default(),
                run_immediately: Some(false),
                kind: "authenticity".into(),
                authenticity: None,
            },
        )
        .await
        .unwrap()
        .0
    }

    async fn deep_check(&self, monitor: Uuid, claimed: &str, samples: usize) -> Value {
        let input = StartInput {
            claimed_model: claimed.into(),
            samples,
            early_stop: true,
            concurrency: 2,
            acknowledge_risk: true,
        };
        let (job, _) = deep::enqueue(&self.state.db, self.account, self.admin, monitor, &input)
            .await
            .unwrap();
        assert_eq!(monitor::claim_job(&self.state).await.unwrap(), Some(job));
        monitor::execute_job(&self.state, job).await.unwrap();
        let row: (String, Option<String>, Value, bool, i32) = sqlx::query_as(
            "SELECT status,verdict,evidence,stopped_early,completed_samples FROM quality_monitor_jobs WHERE id=$1",
        )
        .bind(job)
        .fetch_one(&self.state.db)
        .await
        .unwrap();
        json!({"status":row.0,"verdict":row.1,"evidence":row.2,"stopped_early":row.3,"completed":row.4})
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn baseline_import_merge_exclude_and_activate(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let plan = f.plan().await;
    let first = f.bundle(plan, "acme-big", "acme-big", 50, 1);
    let view = f.import(&first).await;
    assert_eq!(view["validation"]["fingerprint_valid"], 50);
    assert_eq!(view["validation"]["compatible_version_id"], Value::Null);
    assert!(view["preview"].is_object());
    // Re-uploading the same bundle is idempotent.
    let (created, again) = baseline::import_bundle(&db, f.admin, &first).await.unwrap();
    assert!(!created);
    assert_eq!(again["id"], view["id"]);

    let first_id: Uuid = serde_json::from_value(view["id"].clone()).unwrap();
    assert!(matches!(
        baseline::decide(&db, f.admin, first_id, Decision::Merge).await,
        Err(AppError::Conflict(_))
    ));
    let version = baseline::decide(&db, f.admin, first_id, Decision::NewVersion)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        baseline::decide(&db, f.admin, first_id, Decision::Reject).await,
        Err(AppError::Conflict(_))
    ));
    let (revision, valid): (i32, i32) =
        sqlx::query_as("SELECT revision,valid_samples FROM model_baseline_versions WHERE id=$1")
            .bind(version)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!((revision, valid), (1, 50));

    // Same conditions and distribution: merges incrementally.
    let second_bytes = f.bundle(plan, "acme-big", "acme-big", 40, 2);
    let second = f.import(&second_bytes).await;
    assert_eq!(second["compatible_revision"], 1);
    assert_eq!(second["counts"]["fingerprint"]["valid"], 40);
    assert_eq!(
        second["validation"]["compatible_version_id"],
        json!(version)
    );
    assert_eq!(second["drift"]["flagged"], false);
    let second_id: Uuid = serde_json::from_value(second["id"].clone()).unwrap();
    assert_eq!(
        baseline::decide(&db, f.admin, second_id, Decision::Merge)
            .await
            .unwrap(),
        Some(version)
    );
    let (_, merged) = baseline::import_bundle(&db, f.admin, &second_bytes)
        .await
        .unwrap();
    assert_eq!(merged["version_id"], json!(version));
    assert_eq!(
        merged["version"],
        json!({"revision":2,"status":"draft","valid_samples":90})
    );
    // A batch that behaves like another model is flagged and cannot be merged.
    let drifted = f
        .import(&f.bundle(plan, "acme-big", "zeta-one", 40, 3))
        .await;
    assert_eq!(drifted["drift"]["flagged"], true);
    let drifted_id: Uuid = serde_json::from_value(drifted["id"].clone()).unwrap();
    assert!(matches!(
        baseline::decide(&db, f.admin, drifted_id, Decision::Merge).await,
        Err(AppError::Conflict(_))
    ));
    baseline::decide(&db, f.admin, drifted_id, Decision::Reject)
        .await
        .unwrap();

    // Excluding a batch recomputes the version from the remaining ones.
    let version_after = baseline::exclude_batch(&db, second_id).await.unwrap();
    assert_eq!(version_after, version);
    let (revision, valid): (i32, i32) =
        sqlx::query_as("SELECT revision,valid_samples FROM model_baseline_versions WHERE id=$1")
            .bind(version)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!((revision, valid), (3, 50));
    assert!(matches!(
        baseline::exclude_batch(&db, second_id).await,
        Err(AppError::Conflict(_))
    ));

    baseline::activate_version(&db, version).await.unwrap();
    let overview = baseline::overview(&db).await.unwrap();
    let model = overview["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["model"] == "acme-big")
        .unwrap();
    let progress = &model["active"]["progress"];
    assert_eq!(progress["batches"], 1, "{progress}");
    assert_eq!(
        progress["token_counted"].as_i64().unwrap() + progress["token_missing"].as_i64().unwrap(),
        progress["token_target"].as_i64().unwrap()
    );
    let evaluation = baseline::evaluate_version(&db, version)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evaluation["samples"], 50);

    // Collection conditions must match the plan and target.
    let mut manifest = bundle::test_manifest(plan, "acme-big", 1);
    manifest["provider_only"] = json!("other");
    let wrong = bundle::build(manifest, &[]);
    assert!(matches!(
        baseline::import_bundle(&db, f.admin, &wrong).await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("UPDATE model_baseline_plans SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(plan)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        baseline::import_bundle(&db, f.admin, &f.bundle(plan, "acme-big", "acme-big", 10, 4)).await,
        Err(AppError::BadRequest(_))
    ));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn default_templates_import_once_and_publish(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let total = super::defaults::all().count();
    assert_eq!(
        baseline::import_default_templates(&f.state, f.admin)
            .await
            .unwrap(),
        total
    );
    assert_eq!(
        baseline::import_default_templates(&f.state, f.admin)
            .await
            .unwrap(),
        0
    );
    // A retired default stays retired on a later import.
    sqlx::query("UPDATE probe_templates SET retired_at=now() WHERE id=(SELECT id FROM probe_templates ORDER BY created_at DESC LIMIT 1)")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        baseline::import_default_templates(&f.state, f.admin)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        baseline::publish_pool_version(&db, f.admin).await.unwrap(),
        2
    );
    let templates = baseline::pool_templates(&f.state, 2).await.unwrap();
    assert_eq!(templates.len(), 3 + total - 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn models_with_mandatory_reasoning_use_their_default_reasoning(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    baseline::upsert_target(
        &db,
        "acme-big",
        &TargetInput {
            family: "acme".into(),
            openrouter_model: "vendor/acme-big".into(),
            provider_only: "acme".into(),
            target_samples: 60,
            token_samples: 4,
            reasoning: "default".into(),
        },
    )
    .await
    .unwrap();
    let plan = f.plan().await;
    let file = baseline::plan_file(&f.state, plan).await.unwrap();
    let item = file["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["model"] == "acme-big")
        .unwrap();
    assert_eq!(item["reasoning"], "default");
    let thinking = json!({"sample_id":"t","probe_kind":"fingerprint","template_id":f.fingerprint_template,
        "n":100,"values":(1..=100).collect::<Vec<_>>(),"input_tokens":40,"output_tokens":900,
        "reasoning_tokens":600,"served_provider":"Acme","finish_reason":"stop"});
    // A bundle collected with reasoning off no longer matches this model.
    assert!(matches!(
        baseline::import_bundle(
            &db,
            f.admin,
            &bundle::build(
                bundle::test_manifest(plan, "acme-big", 1),
                std::slice::from_ref(&thinking)
            )
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let mut manifest = bundle::test_manifest(plan, "acme-big", 1);
    manifest["reasoning"] = json!("default");
    let view = f.import(&bundle::build(manifest, &[thinking])).await;
    assert_eq!(view["valid_count"], 1, "{view}");
    let id: Uuid = serde_json::from_value(view["id"].clone()).unwrap();
    let version = baseline::decide(&db, f.admin, id, Decision::NewVersion)
        .await
        .unwrap()
        .unwrap();
    let reasoning: String =
        sqlx::query_scalar("SELECT reasoning FROM model_baseline_versions WHERE id=$1")
            .bind(version)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(reasoning, "default");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn import_rejects_untrusted_samples(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    let plan = f.plan().await;
    let sample = |id: &str, provider: &str, reasoning: i64, template: Uuid| {
        json!({"sample_id":id,"probe_kind":"fingerprint","template_id":template,"n":100,
            "values":(1..=100).collect::<Vec<_>>(),"input_tokens":40,"output_tokens":300,
            "reasoning_tokens":reasoning,"served_provider":provider,"finish_reason":"stop"})
    };
    let bytes = bundle::build(
        bundle::test_manifest(plan, "acme-big", 1),
        &[
            sample("ok", "Acme", 0, f.fingerprint_template),
            sample("bedrock", "Amazon Bedrock", 0, f.fingerprint_template),
            sample("thinking", "Acme", 900, f.fingerprint_template),
            sample("unknown", "Acme", 0, Uuid::new_v4()),
            {
                let mut s = sample("silent", "", 0, f.fingerprint_template);
                s.as_object_mut().unwrap().remove("served_provider");
                s
            },
        ],
    );
    let view = f.import(&bytes).await;
    assert_eq!(view["valid_count"], 1);
    assert_eq!(
        view["validation"]["invalid"],
        json!({"no_provider":1,"provider_mismatch":1,"reasoning":1,"unknown_template":1})
    );
    assert_eq!(
        view["counts"],
        json!({"fingerprint":{"total":5,"valid":1,"invalid":
            {"no_provider":1,"provider_mismatch":1,"reasoning":1,"unknown_template":1}}})
    );
    assert_eq!(view["version"], Value::Null);
    // Imports from before the split stored a missing provider as a mismatch.
    sqlx::query("UPDATE model_baseline_samples SET invalid_reason='provider_mismatch' WHERE sample_id='silent'")
        .execute(&db)
        .await
        .unwrap();
    let (_, again) = baseline::import_bundle(&db, f.admin, &bytes).await.unwrap();
    assert_eq!(again["counts"], view["counts"]);
}

// Authenticity checks are a hosted-service feature.
#[cfg(feature = "cloud")]
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn deep_check_detects_substitution_and_confirms_genuine_channels(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    f.activate_all().await;
    let substituted = f.channel_monitor("big", "acme-big").await;
    let genuine = f.channel_monitor("zeta", "zeta-one").await;

    // The first check on a channel requires the risk acknowledgement.
    let mut input = StartInput {
        claimed_model: "acme-big".into(),
        samples: 16,
        early_stop: true,
        concurrency: 2,
        acknowledge_risk: false,
    };
    assert!(matches!(
        deep::enqueue(&db, f.account, f.admin, substituted, &input).await,
        Err(AppError::Refused("authenticity_ack_required", _))
    ));
    // Monitors of another account are invisible.
    input.acknowledge_risk = true;
    assert!(matches!(
        deep::enqueue(&db, Uuid::new_v4(), f.admin, substituted, &input).await,
        Err(AppError::NotFound)
    ));
    input.claimed_model = "unknown-model".into();
    assert!(matches!(
        deep::enqueue(&db, f.account, f.admin, substituted, &input).await,
        Err(AppError::BadRequest(_))
    ));

    // "acme-big" served as zeta-one: different numbers and a different tokenizer.
    let result = f.deep_check(substituted, "acme-big", 16).await;
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["verdict"], "confirmed", "{result}");
    assert_eq!(result["evidence"]["best_model"], "zeta-one");
    assert_eq!(result["evidence"]["tokenizer"]["consistent"], false);

    let result = f.deep_check(genuine, "zeta-one", 32).await;
    assert_eq!(result["verdict"], "match", "{result}");
    assert_eq!(result["stopped_early"], true);
    assert!(result["completed"].as_i64().unwrap() < 32);
    let identity: Option<String> =
        sqlx::query_scalar("SELECT identity_verdict FROM quality_monitors WHERE id=$1")
            .bind(genuine)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(identity.as_deref(), Some("match"));
    // Deep checks never show up as the monitor's v2 job status.
    let monitors = qm::load(&db, f.account).await.unwrap();
    assert!(monitors.iter().all(|m| m.job_status.is_none()));

    // The account history lists both checks, newest first, and never another account's.
    let all = deep::history(&db, f.account, &serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(all["total"], 2);
    assert_eq!(all["items"][0]["verdict"], "match");
    assert_eq!(all["items"][1]["channel_name"], "relay big");
    let other = deep::history(
        &db,
        Uuid::new_v4(),
        &serde_json::from_value(json!({})).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(other["total"], 0);

    // The daily sample budget includes samples already taken today.
    sqlx::query("UPDATE quality_monitors SET daily_sample_limit=40 WHERE id=$1")
        .bind(genuine)
        .execute(&db)
        .await
        .unwrap();
    input.claimed_model = "zeta-one".into();
    input.samples = 32;
    assert!(matches!(
        deep::enqueue(&db, f.account, f.admin, genuine, &input).await,
        Err(AppError::Conflict(_))
    ));
}

// Authenticity checks are a hosted-service feature.
#[cfg(feature = "cloud")]
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn scheduled_sampling_judges_a_window_alerts_and_excludes(db: PgPool) {
    use super::schedule;
    let f = Fixture::new(db.clone()).await;
    f.activate_all().await;
    let monitor = f.channel_monitor("big", "acme-big").await;
    let channel: Uuid = sqlx::query_scalar("SELECT channel_id FROM quality_monitors WHERE id=$1")
        .bind(monitor)
        .fetch_one(&db)
        .await
        .unwrap();
    // The claim defaults from the channel's upstream model.
    schedule::set_default_claim(&db, f.account, monitor)
        .await
        .unwrap();
    let claimed: Option<String> =
        sqlx::query_scalar("SELECT claimed_model FROM quality_monitors WHERE id=$1")
            .bind(monitor)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(claimed.as_deref(), Some("acme-big"));

    let enable = json!({"schedule_enabled":true,"samples_per_run":8,"route_policy":"exclude"});
    assert!(matches!(
        schedule::patch(&db, f.account, f.admin, monitor, &enable).await,
        Err(AppError::Refused("authenticity_ack_required", _))
    ));
    assert!(matches!(
        schedule::patch(
            &db,
            f.account,
            f.admin,
            monitor,
            &json!({"claimed_model":"no-baseline"})
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let mut acknowledged = enable.clone();
    acknowledged["acknowledge_risk"] = json!(true);
    schedule::patch(&db, f.account, f.admin, monitor, &acknowledged)
        .await
        .unwrap();
    // Service settings never apply to authenticity monitors and vice versa.
    assert!(matches!(
        qm::enqueue(&db, f.account, monitor, "manual").await,
        Err(AppError::Conflict(_))
    ));

    let run = || async {
        sqlx::query("UPDATE quality_monitors SET next_run_at=now() WHERE id=$1")
            .bind(monitor)
            .execute(&db)
            .await
            .unwrap();
        monitor::schedule_due(&f.state).await.unwrap();
        let job = monitor::claim_job(&f.state).await.unwrap().unwrap();
        monitor::execute_job(&f.state, job).await.unwrap();
        sqlx::query_as::<_, (Option<String>, Value)>(
            "SELECT window_verdict,window_evidence FROM quality_monitors WHERE id=$1",
        )
        .bind(monitor)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    let (first, evidence) = run().await;
    // Eight samples are too few to confirm a cross-vendor substitution.
    assert_ne!(first.as_deref(), Some("confirmed"), "{evidence}");
    assert_eq!(evidence["window"]["samples"], 8);
    let (_, evidence) = run().await;
    assert_eq!(evidence["window"]["samples"], 16);
    let (verdict, evidence) = run().await;
    assert_eq!(verdict.as_deref(), Some("confirmed"), "{evidence}");
    assert_eq!(evidence["window"]["samples"], 24);
    // Token counts are deterministic: only the first run probed them.
    let token_probes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticity_samples WHERE monitor_id=$1 AND probe_kind='token'",
    )
    .bind(monitor)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(token_probes, 2);
    // The alert is recorded once per new verdict (SMTP is not configured here).
    let alerted: Option<String> =
        sqlx::query_scalar("SELECT alerted_verdict FROM quality_monitors WHERE id=$1")
            .bind(monitor)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(alerted.as_deref(), Some("confirmed"));
    // A confirmed mismatch with the exclude policy removes the model from routing.
    let scopes = crate::models::scopes(&db, f.account).await.unwrap();
    let scope = scopes.iter().find(|s| s.id == channel).unwrap();
    assert_eq!(scope.excluded_models, vec!["big".to_owned()]);
    // Changing the claim starts a fresh window and lifts the routing effect.
    schedule::patch(
        &db,
        f.account,
        f.admin,
        monitor,
        &json!({"claimed_model":"zeta-one"}),
    )
    .await
    .unwrap();
    let scopes = crate::models::scopes(&db, f.account).await.unwrap();
    assert!(
        scopes
            .iter()
            .find(|s| s.id == channel)
            .unwrap()
            .excluded_models
            .is_empty()
    );
    let history = deep::history(&db, f.account, &serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(history["total"], 3);
    assert_eq!(history["items"][0]["source"], "scheduled");
}

// Authenticity checks are a hosted-service feature.
#[cfg(feature = "cloud")]
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn authenticity_monitors_are_created_with_their_settings(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    f.activate_all().await;
    let channel: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,models,model_mapping)
         VALUES($1,'relay new','openai','http://1.1.1.1','[\"big\"]','{\"big\":\"acme-big\"}') RETURNING id",
    )
    .bind(f.account)
    .fetch_one(&db)
    .await
    .unwrap();
    let input = |settings: Value| CreateInput {
        channel_id: channel,
        model: "big".into(),
        settings: Settings::default(),
        run_immediately: None,
        kind: "authenticity".into(),
        authenticity: Some(settings),
    };
    let settings = json!({"claimed_model":"acme-big","schedule_enabled":true,"interval_seconds":10800,"route_policy":"deprioritize"});
    // A refused setting leaves no monitor behind, so the add can be retried.
    assert!(matches!(
        qm::create_monitor(&db, f.account, f.admin, &input(settings.clone())).await,
        Err(AppError::Refused("authenticity_ack_required", _))
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM quality_monitors WHERE channel_id=$1")
            .bind(channel)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let mut acknowledged = settings;
    acknowledged["acknowledge_risk"] = json!(true);
    let (id, job) = qm::create_monitor(&db, f.account, f.admin, &input(acknowledged))
        .await
        .unwrap();
    assert_eq!(job, None);
    let row: (Option<String>, bool, i32, String, bool) = sqlx::query_as(
        "SELECT claimed_model,schedule_enabled,interval_seconds,route_policy,next_run_at<=now() FROM quality_monitors WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            Some("acme-big".into()),
            true,
            10800,
            "deprioritize".into(),
            true
        )
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn public_check_reports_without_keeping_the_key(db: PgPool) {
    let f = Fixture::new(db.clone()).await;
    f.activate_all().await;
    let state = f.state.clone();

    // Unsupported models and missing risk acknowledgement fail before any request.
    let input = |model: &str, accept: bool| {
        Json(
            serde_json::from_value::<public::StartInput>(json!({
                "protocol":"openai","base_url":"https://api.example.com/v1","api_key":"sk-test",
                "model":model,"email":"public@example.invalid",
                "request_id":Uuid::new_v4(),"code":"000000","accept_risks":accept,
            }))
            .unwrap(),
        )
    };
    let start = |model: &str, accept: bool| {
        public::start(
            axum::extract::State(state.clone()),
            axum::http::HeaderMap::new(),
            input(model, accept),
        )
    };
    assert!(matches!(
        start("acme-big", false).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        start("unknown-model", true).await,
        Err(AppError::BadRequest(_))
    ));
    // A code that was never requested is rejected.
    assert!(matches!(
        start("acme-big", true).await,
        Err(AppError::BadRequest(_))
    ));

    // "acme-big" served as zeta-one, queued as start() would leave it.
    let (id, token, manage) = (Uuid::new_v4(), "vr_test_report", "vm_test_manage");
    public::test_enqueue(
        &state,
        id,
        token,
        manage,
        "acme-big",
        "http://1.1.1.1",
        "authenticity-test-only-credential",
    )
    .await;
    public::run_next(&state).await;

    let report = public::get(
        axum::extract::State(state.clone()),
        Path(token.into()),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(report["status"], "done", "{report}");
    assert_eq!(report["verdict"], "confirmed", "{report}");
    assert_eq!(report["host"], Value::Null);
    assert_eq!(report["evidence"]["closest_model"], "zeta-one");
    assert_eq!(report["evidence"]["vendor"]["consistent"], false);
    assert_eq!(report["evidence"]["shares"][0]["model"], "zeta-one");
    let rows = report["evidence"]["samples"].as_array().unwrap();
    assert_eq!(
        rows.len() as i64,
        report["completed_samples"].as_i64().unwrap() + 2
    );
    assert!(rows.iter().any(|r| r["leaning"] == "zeta-one"));
    // Neither the report nor the database keeps the key or the full URL.
    let stored: String =
        sqlx::query_scalar("SELECT row_to_json(p)::text FROM public_checks p WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(!stored.contains("authenticity-test-only-credential"));
    assert!(!report.to_string().contains("1.1.1.1"));
    assert!(!public::secret_exists(&state, id).await);

    // The creator sees the host; others only after it is shared.
    let mut owner = HeaderMap::new();
    owner.insert("x-manage-token", manage.parse().unwrap());
    let mine = public::get(
        axum::extract::State(state.clone()),
        Path(token.into()),
        owner.clone(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(mine["host"], "1.1.1.1");
    assert_eq!(mine["owner"], true);
    let mut stranger = HeaderMap::new();
    stranger.insert("x-manage-token", "vm_wrong".parse().unwrap());
    let share = |headers: HeaderMap, show_host: bool| {
        public::update(
            axum::extract::State(state.clone()),
            Path(token.into()),
            headers,
            Json(serde_json::from_value(json!({ "show_host": show_host })).unwrap()),
        )
    };
    assert!(matches!(
        share(stranger, true).await,
        Err(AppError::NotFound)
    ));
    share(owner, true).await.unwrap();
    let shared = public::get(
        axum::extract::State(state.clone()),
        Path(token.into()),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(shared["host"], "1.1.1.1");
    assert_eq!(shared["owner"], false);

    // Only the manage token deletes the report.
    let delete = |manage: &str| {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-manage-token", manage.parse().unwrap());
        public::delete(
            axum::extract::State(state.clone()),
            Path(token.into()),
            headers,
        )
    };
    assert!(matches!(delete("vm_wrong").await, Err(AppError::NotFound)));
    delete(manage).await.unwrap();
    assert!(matches!(
        public::get(
            axum::extract::State(state.clone()),
            Path(token.into()),
            HeaderMap::new()
        )
        .await,
        Err(AppError::NotFound)
    ));
    let samples: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public_check_samples WHERE check_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(samples, 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-authenticity.sh"]
async fn public_check_capacity_falls_back_to_defaults(db: PgPool) {
    assert_eq!(
        public::load_capacity(&db).await,
        public::Capacity::default()
    );
    let store = |value: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query(
                "INSERT INTO system_settings(key,value_ciphertext) VALUES ('public_verify',$1)
                 ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext",
            )
            .bind(value)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    store(r#"{"workers":8,"concurrency":3,"max_waiting":60}"#).await;
    assert_eq!(
        public::load_capacity(&db).await,
        public::Capacity {
            workers: 8,
            concurrency: 3,
            max_waiting: 60
        }
    );
    // An out-of-range or unreadable value never stops public checks.
    store(r#"{"workers":0,"concurrency":3,"max_waiting":60}"#).await;
    assert_eq!(
        public::load_capacity(&db).await,
        public::Capacity::default()
    );
    store("not json").await;
    assert_eq!(
        public::load_capacity(&db).await,
        public::Capacity::default()
    );
}
