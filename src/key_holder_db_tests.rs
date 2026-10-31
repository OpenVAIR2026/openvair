use super::*;
use axum::{body::Body, body::to_bytes, http::Request};
use sqlx::PgPool;
use std::sync::Arc;

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-key-holder.sh"]
async fn request_details_are_key_scoped_and_never_return_channels_or_attempts(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "detail owner").await;
    let other = account(&state.db, "other owner").await;
    let upstream = channel(&state, owner, "gpt-a").await;
    let (key_id, raw) = key(&state.db, owner, json!([])).await;
    let (_, sibling_raw) = key(&state.db, owner, json!([])).await;
    let (_, foreign_raw) = key(&state.db, other, json!([])).await;
    usage_event(&state.db, owner, key_id, upstream, 200).await;
    let event: Uuid =
        sqlx::query_scalar("SELECT id FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2")
            .bind(owner)
            .bind(key_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let diagnostics = json!({"queue_ms":10,"completion":"success","attempts":[{
        "channel_id":upstream,"channel_name":"owner upstream","started_ms":20,"duration_ms":100,
        "headers_ms":30,"first_token_ms":40,"status":200,"outcome":"success"
    }]});
    sqlx::query("UPDATE usage_events SET request_diagnostics=$3,stream=true,first_token_ms=60 WHERE account_id=$1 AND id=$2")
        .bind(owner).bind(event).bind(diagnostics).execute(&state.db).await.unwrap();
    let headers = || holder_headers(&raw, "198.51.100.54");
    let response = detail(State(state.clone()), Path(event), headers())
        .await
        .unwrap();
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    let (_, body) = read(response).await;
    assert_eq!(body["timing"]["queue_ms"], 10);
    assert_eq!(body["timing"]["upstream_headers_ms"], 30);
    assert_eq!(body["timing"]["headers_to_first_token_ms"], 10);
    assert_eq!(body["first_token_ms"], 60);
    let serialized = body.to_string();
    for forbidden in [
        "channel",
        "attempt",
        "request_diagnostics",
        "secret-upstream-model",
        "price_snapshot",
        "owner upstream",
        &upstream.to_string(),
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    for forbidden_key in [sibling_raw, foreign_raw] {
        assert!(matches!(
            detail(
                State(state.clone()),
                Path(event),
                holder_headers(&forbidden_key, "198.51.100.54")
            )
            .await,
            Err(AppError::NotFound)
        ));
    }
    sqlx::query("UPDATE usage_events SET request_diagnostics=NULL WHERE account_id=$1 AND id=$2")
        .bind(owner)
        .bind(event)
        .execute(&state.db)
        .await
        .unwrap();
    let (_, legacy) = read(
        detail(State(state.clone()), Path(event), headers())
            .await
            .unwrap(),
    )
    .await;
    assert!(legacy["timing"].is_null());
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE account_id=$1 AND id=$2")
        .bind(owner)
        .bind(key_id)
        .execute(&state.db)
        .await
        .unwrap();
    assert!(
        detail(State(state.clone()), Path(event), headers())
            .await
            .is_ok()
    );
    sqlx::query("UPDATE virtual_api_keys SET revoked_at=now() WHERE account_id=$1 AND id=$2")
        .bind(owner)
        .bind(key_id)
        .execute(&state.db)
        .await
        .unwrap();
    let response = detail(State(state), Path(event), headers())
        .await
        .err()
        .unwrap()
        .into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

fn test_state(db: PgPool) -> AppState {
    AppState {
        db,
        redis: redis::Client::open(
            std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into()),
        )
        .unwrap(),
        http: reqwest::Client::new(),
        crypto: Arc::new(crate::crypto::Crypto::from_env().unwrap()),
        semaphores: Arc::default(),
        cookie_secure: false,
        super_admin_emails: Arc::default(),
        redis_connection: Default::default(),
        routing_settings: Default::default(),
        prices: Default::default(),
    }
}

async fn account(db: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO accounts(name) VALUES ($1) RETURNING id")
        .bind(name)
        .fetch_one(db)
        .await
        .unwrap()
}

/// An upstream OpenAI channel on `account` that serves `model`.
async fn channel(state: &AppState, account: Uuid, model: &str) -> Uuid {
    sqlx::query(
        "INSERT INTO model_catalog(pattern,label) VALUES ($1,$1) ON CONFLICT (pattern) DO NOTHING",
    )
    .bind(model)
    .execute(&state.db)
    .await
    .unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,models)
         VALUES ($1,'owner upstream','openai','http://1.1.1.1/v1',$2) RETURNING id",
    )
    .bind(account)
    .bind(json!([model]))
    .fetch_one(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,'test',$2)",
    )
    .bind(id)
    .bind(state.crypto.encrypt("upstream-test-only").unwrap())
    .execute(&state.db)
    .await
    .unwrap();
    id
}

async fn key(db: &PgPool, account: Uuid, allowed: Value) -> (Uuid, String) {
    let raw = auth::random_token("vair");
    let mode = if allowed.as_array().is_some_and(|rules| rules.is_empty()) {
        "all"
    } else {
        "allowlist"
    };
    let id = sqlx::query_scalar(
        "INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models,model_mode,token_limit_daily)
         VALUES ($1,'holder key','vair_test',$2,$3,$4,1000) RETURNING id",
    )
    .bind(account)
    .bind(auth::hash_token(&raw))
    .bind(allowed)
    .bind(mode)
    .fetch_one(db)
    .await
    .unwrap();
    (id, raw)
}

async fn usage_event(db: &PgPool, account: Uuid, key: Uuid, channel: Uuid, status: i32) {
    sqlx::query(
        "INSERT INTO usage_events(account_id,virtual_key_id,channel_id,model,upstream_model,protocol,status,
             duration_ms,input_tokens,output_tokens,cost_usd,pricing_status)
         VALUES ($1,$2,$3,'gpt-a','secret-upstream-model','openai',$4,120,300,200,0.25,'priced')",
    )
    .bind(account)
    .bind(key)
    .bind(channel)
    .bind(status)
    .execute(db)
    .await
    .unwrap();
}

fn holder_headers(raw: &str, ip: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {raw}").parse().unwrap());
    headers.insert("x-forwarded-for", ip.parse().unwrap());
    headers
}

async fn read(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_summary(state: &AppState, headers: HeaderMap) -> (StatusCode, Value) {
    match summary(State(state.clone()), headers).await {
        Ok(response) => read(response).await,
        Err(error) => read(error.into_response()).await,
    }
}

async fn get_events(state: &AppState, headers: HeaderMap, query: Value) -> (StatusCode, Value) {
    let query = Query(serde_json::from_value(query).unwrap());
    match events(State(state.clone()), headers, query).await {
        Ok(response) => read(response).await,
        Err(error) => read(error.into_response()).await,
    }
}

async fn chat(state: &AppState, raw: &str, model: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("host", "vair.example.invalid")
        .header("authorization", format!("Bearer {raw}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model": model, "messages": []}).to_string(),
        ))
        .unwrap();
    read(gateway::handle(State(state.clone()), request).await).await
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL, Redis and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn summary_shows_status_limits_and_models_without_upstream_details(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "Lab").await;
    channel(&state, owner, "gpt-b").await;
    let channel = channel(&state, owner, "gpt-a").await;
    let (key_id, raw) = key(&state.db, owner, json!(["gpt-a"])).await;
    usage_event(&state.db, owner, key_id, channel, 200).await;

    let (status, view) = get_summary(&state, holder_headers(&raw, "198.51.100.1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["account_name"], "Lab");
    assert_eq!(view["id"], json!(key_id));
    assert_eq!(view["key_name"], "holder key");
    assert_eq!(view["status"], "active");
    assert_eq!(view["token_limit_daily"], 1000);
    assert_eq!(view["used_tokens_today"], 500);
    assert_eq!(view["used_cost_today_usd"], 0.25);
    assert_eq!(view["models"], json!({"openai": ["gpt-a"]}));
    assert!(view["last_used_at"].is_string());
    let text = view.to_string();
    for private in ["1.1.1.1", "owner upstream", "secret-upstream-model"] {
        assert!(!text.contains(private), "{text}");
    }

    // Reaching the daily token limit shows up as the key's status.
    usage_event(&state.db, owner, key_id, channel, 200).await;
    let (_, view) = get_summary(&state, holder_headers(&raw, "198.51.100.1")).await;
    assert_eq!(view["status"], "exhausted_daily");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL, Redis and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn paused_keys_explain_themselves_and_revoked_keys_do_not(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "Lab").await;
    channel(&state, owner, "gpt-a").await;
    let (key_id, raw) = key(&state.db, owner, json!([])).await;
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(key_id)
        .execute(&state.db)
        .await
        .unwrap();

    let (status, view) = get_summary(&state, holder_headers(&raw, "198.51.100.2")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["status"], "paused");
    assert_eq!(view["models"], json!({}));
    let (status, body) = chat(&state, &raw, "gpt-a").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "key_paused");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("https://vair.example.invalid/my-key"),
        "{message}"
    );

    // A suspended owner account pauses the key too.
    sqlx::query("UPDATE virtual_api_keys SET enabled=true WHERE id=$1")
        .bind(key_id)
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE accounts SET disabled_at=now() WHERE id=$1")
        .bind(owner)
        .execute(&state.db)
        .await
        .unwrap();
    let (_, view) = get_summary(&state, holder_headers(&raw, "198.51.100.2")).await;
    assert_eq!(view["status"], "paused");

    sqlx::query("UPDATE virtual_api_keys SET revoked_at=now() WHERE id=$1")
        .bind(key_id)
        .execute(&state.db)
        .await
        .unwrap();
    for (status, body) in [
        get_summary(&state, holder_headers(&raw, "198.51.100.2")).await,
        get_summary(&state, holder_headers("vair_unknown", "198.51.100.2")).await,
        chat(&state, &raw, "gpt-a").await,
    ] {
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "invalid_api_key");
        assert!(!body.to_string().contains("my-key"), "{body}");
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL, Redis and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn events_list_only_this_keys_requests(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "Lab").await;
    let other = account(&state.db, "Other").await;
    let owner_channel = channel(&state, owner, "gpt-a").await;
    let other_channel = channel(&state, other, "gpt-a").await;
    let (key_id, raw) = key(&state.db, owner, json!([])).await;
    let (sibling, _) = key(&state.db, owner, json!([])).await;
    let (foreign, _) = key(&state.db, other, json!([])).await;
    usage_event(&state.db, owner, key_id, owner_channel, 200).await;
    usage_event(&state.db, owner, key_id, owner_channel, 502).await;
    usage_event(&state.db, owner, sibling, owner_channel, 200).await;
    usage_event(&state.db, other, foreign, other_channel, 200).await;

    let headers = || holder_headers(&raw, "198.51.100.3");
    let (status, page) = get_events(&state, headers(), json!({"range": "7d"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 2);
    let first = &page["items"][0];
    assert_eq!(first["model"], "gpt-a");
    assert_eq!(first["cost_usd"], 0.25);
    for hidden in [
        "channel_id",
        "channel_name",
        "upstream_model",
        "price_snapshot",
    ] {
        assert!(first.get(hidden).is_none(), "{first}");
    }
    let (_, errors) = get_events(&state, headers(), json!({"status": "error"})).await;
    assert_eq!(errors["total"], 1);
    assert_eq!(errors["items"][0]["status"], 502);
    let (status, _) = get_events(&state, headers(), json!({"status": "bogus"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let query = Query(serde_json::from_value(json!({"range": "30d"})).unwrap());
    let (status, summary) =
        read(usage(State(state.clone()), headers(), query).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["totals"]["requests"], 2);
    assert_eq!(summary["totals"]["successful"], 1);
    assert_eq!(summary["totals"]["known_cost_usd"], 0.5);
    assert_eq!(summary["models"][0]["model"], "gpt-a");
    assert_eq!(summary["trend"].as_array().unwrap().len(), 30);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL, Redis and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn gateway_names_the_rule_that_stopped_a_request(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "Lab").await;
    let channel = channel(&state, owner, "gpt-a").await;
    let (key_id, raw) = key(&state.db, owner, json!(["gpt-a", "gpt-z"])).await;

    let (status, body) = chat(&state, &raw, "gpt-b").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "model_not_allowed");

    let (status, body) = chat(&state, &raw, "gpt-z").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "model_unavailable");

    usage_event(&state.db, owner, key_id, channel, 200).await;
    usage_event(&state.db, owner, key_id, channel, 200).await;
    let (status, body) = chat(&state, &raw, "gpt-a").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "daily_token_limit");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("在 https://vair.example.invalid/my-key 查看此 Key 的状态与额度"),
        "{body}"
    );

    sqlx::query(
        "UPDATE virtual_api_keys SET token_limit_daily=NULL,cost_limit_total_usd=0.4 WHERE id=$1",
    )
    .bind(key_id)
    .execute(&state.db)
    .await
    .unwrap();
    let (status, body) = chat(&state, &raw, "gpt-a").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "total_cost_limit");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL, Redis and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn repeated_bad_keys_from_one_address_are_slowed_down(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "Lab").await;
    let (_, raw) = key(&state.db, owner, json!([])).await;
    let ip = format!("203.0.113.{}", rand::random::<u8>());
    for _ in 0..FAILURE_LIMIT_PER_IP {
        let (status, _) = get_summary(&state, holder_headers("vair_wrong", &ip)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = get_summary(&state, holder_headers(&raw, &ip)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // Other addresses are unaffected.
    let (status, _) = get_summary(&state, holder_headers(&raw, "198.51.100.9")).await;
    assert_eq!(status, StatusCode::OK);
}
