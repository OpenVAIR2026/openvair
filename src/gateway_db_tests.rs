use super::*;
use axum::{Router, routing::any};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[path = "unknown_cost_db_tests.rs"]
mod unknown_cost;

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn request_diagnostics_measure_queue_failover_and_enforce_detail_account(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let failed = channel(&state, own, 10, "passthrough", json!([]), "/missing").await;
    let good = channel(&state, own, 20, "passthrough", json!([]), "/delayed").await;
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    sqlx::query("UPDATE virtual_api_keys SET concurrency_limit=1 WHERE id=$1")
        .bind(key_id)
        .execute(&db)
        .await
        .unwrap();
    let semaphore = state.semaphore(key_id, 1).await;
    let permit = semaphore.acquire_owned().await.unwrap();
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        handle_inner(task_state, request(&raw, "fixture", true))
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    drop(permit);
    let response = task.await.unwrap();
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let (event, timing, first, total): (Uuid, sqlx::types::Json<crate::request_diagnostics::Diagnostics>, i32, i32) = sqlx::query_as(
        "SELECT id,request_diagnostics,first_token_ms,duration_ms FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2")
        .bind(own).bind(key_id).fetch_one(&db).await.unwrap();
    assert!(timing.queue_ms.unwrap() >= 30);
    assert_eq!(timing.completion, Some(Outcome::Success));
    assert_eq!(timing.attempts.len(), 2);
    assert_eq!(timing.attempts[0].channel_id, failed);
    assert_eq!(timing.attempts[0].status, Some(404));
    assert_eq!(timing.attempts[0].outcome, Outcome::HttpError);
    let last = &timing.attempts[1];
    assert_eq!(last.channel_id, good);
    assert!(last.headers_ms.unwrap() >= 30);
    assert!(last.first_token_ms.unwrap() >= last.headers_ms.unwrap() + 40);
    assert_eq!(first, last.started_ms + last.first_token_ms.unwrap());
    assert!(total >= first);
    let owner_headers = headers(&db, own, false).await;
    let Json(detail) = crate::usage::detail(
        State(state.clone()),
        axum::extract::Path(event),
        owner_headers.clone(),
    )
    .await
    .unwrap();
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(
        detail["request_diagnostics"]["attempts"][1]["channel_name"],
        "fixture"
    );
    assert!(!detail.to_string().contains("credential"));
    assert!(!detail.to_string().contains("1.1.1.1"));
    let foreign = headers(&db, other, false).await;
    assert!(matches!(
        crate::usage::detail(State(state.clone()), axum::extract::Path(event), foreign).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        crate::usage::detail(
            State(state.clone()),
            axum::extract::Path(event),
            HeaderMap::new()
        )
        .await,
        Err(AppError::Unauthorized)
    ));
    sqlx::query("UPDATE usage_events SET request_diagnostics=NULL WHERE id=$1 AND account_id=$2")
        .bind(event)
        .bind(own)
        .execute(&db)
        .await
        .unwrap();
    let Json(legacy) =
        crate::usage::detail(State(state), axum::extract::Path(event), owner_headers)
            .await
            .unwrap();
    assert!(serde_json::to_value(legacy).unwrap()["request_diagnostics"].is_null());
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn request_diagnostics_record_timeout_stream_error_and_disconnect(db: PgPool) {
    let (mut state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let channel_id = channel(&state, own, 10, "passthrough", json!([]), "/stream-error").await;
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    let response = handle_inner(state.clone(), request(&raw, "fixture", true))
        .await
        .unwrap();
    assert!(to_bytes(response.into_body(), usize::MAX).await.is_err());
    let completion = |expected: &str| {
        let state = state.clone();
        let expected = expected.to_owned();
        async move {
            for _ in 0..100 {
                let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 AND request_diagnostics->>'completion'=$3 AND in_flight_until IS NULL)")
                    .bind(own).bind(key_id).bind(&expected).fetch_one(&state.db).await.unwrap();
                if found {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("missing completion {expected}");
        }
    };
    completion("stream_error").await;
    sqlx::query(
        "UPDATE channels SET base_url='http://1.1.1.1/body-error' WHERE id=$1 AND account_id=$2",
    )
    .bind(channel_id)
    .bind(own)
    .execute(&db)
    .await
    .unwrap();
    assert!(
        handle_inner(state.clone(), request(&raw, "fixture", false))
            .await
            .is_err()
    );
    let failed_status: i32 = sqlx::query_scalar("SELECT status FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 AND stream=false")
        .bind(own).bind(key_id).fetch_one(&db).await.unwrap();
    assert_eq!(failed_status, 500);
    sqlx::query(
        "UPDATE channels SET base_url='http://1.1.1.1/delayed' WHERE id=$1 AND account_id=$2",
    )
    .bind(channel_id)
    .bind(own)
    .execute(&db)
    .await
    .unwrap();
    let response = handle_inner(state.clone(), request(&raw, "fixture", true))
        .await
        .unwrap();
    drop(response);
    completion("client_disconnected").await;
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (timing, first): (sqlx::types::Json<crate::request_diagnostics::Diagnostics>, Option<i32>) = sqlx::query_as(
        "SELECT request_diagnostics,first_token_ms FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 ORDER BY created_at DESC LIMIT 1")
        .bind(own).bind(key_id).fetch_one(&db).await.unwrap();
    assert_eq!(timing.completion, Some(Outcome::Success));
    assert!(timing.attempts[0].headers_ms.unwrap() >= 30);
    assert!(timing.attempts[0].first_token_ms.is_none());
    assert!(first.is_none());
    // A stalled local proxy times out without sending a credential externally.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stalled = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });
    state.http = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
        .timeout(std::time::Duration::from_millis(50))
        .build()
        .unwrap();
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let outcome: String = sqlx::query_scalar("SELECT request_diagnostics->>'completion' FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 ORDER BY created_at DESC LIMIT 1")
        .bind(own).bind(key_id).fetch_one(&db).await.unwrap();
    assert_eq!(outcome, "timeout");
    stalled.abort();
    server.abort();
}

async fn state(db: PgPool) -> (AppState, tokio::task::JoinHandle<()>) {
    let (state, server, _) = recorded_state(db).await;
    (state, server)
}

async fn recorded_state(
    db: PgPool,
) -> (
    AppState,
    tokio::task::JoinHandle<()>,
    Arc<Mutex<Vec<Value>>>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let router=Router::new().fallback(any(move |request: Request<Body>| {
      let captured = captured.clone();
      async move {
        let path=request.uri().path().to_owned();
        captured.lock().await.push(json!({"url": request.uri().to_string(), "authorization": request.headers().get("authorization").and_then(|v| v.to_str().ok())}));
        if request.method()==axum::http::Method::GET {
            return Json(json!({"data":[{"id":"discovered-private"}]})).into_response();
        }
        let value: Value=serde_json::from_slice(&to_bytes(request.into_body(),usize::MAX).await.unwrap()).unwrap();
        captured.lock().await.last_mut().unwrap()["model"] = value["model"].clone();
        captured.lock().await.last_mut().unwrap()["include_usage"] = value["stream_options"]["include_usage"].clone();
        captured.lock().await.last_mut().unwrap()["messages"] = value["messages"].clone();
        captured.lock().await.last_mut().unwrap()["input"] = value["input"].clone();
        if path.starts_with("/missing/") || value["model"]=="retired-model" {
            return (StatusCode::NOT_FOUND,Json(json!({"error":{"code":"model_not_found","message":"model not found"}}))).into_response();
        }
        if path.starts_with("/bad/") {
            return (StatusCode::BAD_REQUEST,Json(json!({"error":{"code":"invalid_request_error","message":"invalid temperature"}}))).into_response();
        }
        if path.starts_with("/limited/") {
            return (StatusCode::TOO_MANY_REQUESTS,Json(json!({"error":{"code":"rate_limit_exceeded","message":"slow down"}}))).into_response();
        }
        if path.starts_with("/limited-retry/") {
            return (StatusCode::TOO_MANY_REQUESTS,[("retry-after","7")],Json(json!({"error":{"message":"slow down"}}))).into_response();
        }
        if path.starts_with("/quota/") {
            return (StatusCode::TOO_MANY_REQUESTS,Json(json!({"error":{"code":"insufficient_quota","message":"quota exceeded"}}))).into_response();
        }
        if path.starts_with("/credit/") {
            return (StatusCode::BAD_REQUEST,Json(json!({"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low"}}))).into_response();
        }
        if path.starts_with("/denied/") {
            return (StatusCode::UNAUTHORIZED,Json(json!({"error":{"message":"invalid api key"}}))).into_response();
        }
        if path.starts_with("/fail/") {
            return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"message":"upstream overloaded"}}))).into_response();
        }
        if path.starts_with("/delayed/") {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        if path.starts_with("/slow-headers/") {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
        if value["stream"]==true || path.starts_with("/body-error/") {
            if path.starts_with("/delayed/") {
                let chunks = stream::unfold(0, |step| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    match step {
                        0 => Some((Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"cont")), 1)),
                        1 => Some((Ok(axum::body::Bytes::from_static(b"ent\":\"hello\"}}]}\n\ndata: [DONE]\n\n")), 2)),
                        _ => None,
                    }
                });
                return Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(chunks)).unwrap();
            }
            if path.starts_with("/stream-error/") || path.starts_with("/body-error/") {
                let chunks = stream::unfold(0, |step| async move {
                    match step {
                        0 => Some((Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: {}\n\n")), 1)),
                        1 => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            Some((Err(std::io::Error::other("mock interrupted stream")), 2))
                        }
                        _ => None,
                    }
                });
                return Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(chunks)).unwrap();
            }
            if path.starts_with("/slow/") {
                // Headers and a role announcement at once, output only later.
                let chunks = stream::unfold(0, |step| async move {
                    match step {
                        0 => Some((Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n")), 1)),
                        1 => {
                            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                            Some((Ok(axum::body::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\ndata: [DONE]\n\n")), 2))
                        }
                        _ => None,
                    }
                });
                return Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(chunks)).unwrap();
            }
            if path.starts_with("/no-usage/") {
                return ([("content-type", "text/event-stream")], "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n").into_response();
            }
            if path.starts_with("/empty/") {
                return ([("content-type", "text/event-stream")], "data: {}\n\ndata: [DONE]\n\n").into_response();
            }
            if path.starts_with("/late-usage/") {
                // Output at once, usage only after a client may have left.
                let chunks = stream::unfold(0, |step| async move {
                    match step {
                        0 => Some((Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n")), 1)),
                        1 => {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            Some((Ok(axum::body::Bytes::from_static(b"data: {\"usage\":{\"prompt_tokens\":1000000,\"completion_tokens\":100000,\"prompt_tokens_details\":{\"cached_tokens\":200000}}}\n\ndata: [DONE]\n\n")), 2))
                        }
                        _ => None,
                    }
                });
                return Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(chunks)).unwrap();
            }
            return ([("content-type","text/event-stream")],"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":1000000,\"completion_tokens\":100000,\"prompt_tokens_details\":{\"cached_tokens\":200000}}}\n\ndata: [DONE]\n\n").into_response();
        }
        Json(json!({"model":value["model"],"choices":[],"usage":{"prompt_tokens":1000000,"completion_tokens":100000,"prompt_tokens_details":{"cached_tokens":200000}}})).into_response()
    }}));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (
        AppState {
            db,
            redis: redis::Client::open(
                std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into()),
            )
            .unwrap(),
            http: reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(proxy).unwrap())
                .build()
                .unwrap(),
            crypto: Arc::new(crate::crypto::Crypto::from_env().unwrap()),
            semaphores: Arc::new(Mutex::new(HashMap::new())),
            cookie_secure: false,
            super_admin_emails: Arc::default(),
            redis_connection: Default::default(),
            routing_settings: Default::default(),
            prices: Default::default(),
        },
        server,
        requests,
    )
}

async fn account(db: &PgPool) -> Uuid {
    sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('isolated model fixture') RETURNING id")
        .fetch_one(db)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn full_channels_are_passed_over_and_the_last_is_waited_for(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let first = channel(&state, own, 10, "passthrough", json!([]), "").await;
    let second = channel(&state, own, 20, "passthrough", json!([]), "").await;
    for id in [first, second] {
        sqlx::query("UPDATE channels SET concurrency_limit=1,first_token_timeout_seconds=1 WHERE id=$1 AND account_id=$2")
            .bind(id)
            .bind(own)
            .execute(&db)
            .await
            .unwrap();
    }
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    sqlx::query("UPDATE virtual_api_keys SET concurrency_limit=10 WHERE id=$1")
        .bind(key_id)
        .execute(&db)
        .await
        .unwrap();
    let attempts = || async {
        sqlx::query_as::<_, (i32, Value)>(
            "SELECT status,request_diagnostics->'attempts' FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 ORDER BY created_at DESC LIMIT 1")
            .bind(own).bind(key_id).fetch_one(&db).await.unwrap()
    };
    let outcomes = |attempts: &Value| -> Vec<(String, String)> {
        attempts
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["channel_id"].as_str().unwrap().to_owned(),
                    a["outcome"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };

    // The first channel is full, so the request goes to the second.
    let held = state
        .semaphore(first, 1)
        .await
        .acquire_owned()
        .await
        .unwrap();
    assert_eq!(state.in_use(first).await, 1);
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (_, recorded) = attempts().await;
    assert_eq!(
        outcomes(&recorded),
        [
            (first.to_string(), "channel_full".to_owned()),
            (second.to_string(), "success".to_owned())
        ]
    );
    // The permit was released with the finished response.
    assert_eq!(state.in_use(second).await, 0);

    // Both full: the last channel is waited for and used once it frees up.
    let held_second = state
        .semaphore(second, 1)
        .await
        .acquire_owned()
        .await
        .unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(held_second);
    });
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    release.await.unwrap();

    // Still full after the first token timeout: a 503 naming the limit.
    let _held_second = state
        .semaphore(second, 1)
        .await
        .acquire_owned()
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("渠道并发已满"));
    let (status, recorded) = attempts().await;
    assert_eq!(status, 503);
    assert_eq!(
        outcomes(&recorded),
        [
            (first.to_string(), "channel_full".to_owned()),
            (second.to_string(), "channel_full".to_owned())
        ]
    );

    // A streamed response holds its permit until the stream ends.
    drop(held);
    drop(_held_second);
    let response = handle_inner(state.clone(), request(&raw, "fixture", true))
        .await
        .unwrap();
    assert_eq!(state.in_use(first).await, 1);
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    for _ in 0..200 {
        if state.in_use(first).await == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(state.in_use(first).await, 0);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn channel_concurrency_limits_are_validated_cleared_and_shown_per_account(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let id = channel(&state, own, 10, "passthrough", json!([]), "").await;
    let own_headers = headers(&db, own, false).await;
    let other_headers = headers(&db, other, false).await;
    let stored = || async {
        sqlx::query_scalar::<_, Option<i32>>("SELECT concurrency_limit FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
    };
    for invalid in [0, 10001] {
        assert!(matches!(
            patch_channel(
                &state,
                &own_headers,
                id,
                json!({"concurrency_limit": invalid})
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert!(matches!(
        patch_channel(&state, &other_headers, id, json!({"concurrency_limit": 3})).await,
        Err(AppError::NotFound)
    ));
    patch_channel(&state, &own_headers, id, json!({"concurrency_limit": 3}))
        .await
        .unwrap();
    assert_eq!(stored().await, Some(3));
    // Other fields leave the limit alone.
    patch_channel(&state, &own_headers, id, json!({"name": "renamed"}))
        .await
        .unwrap();
    assert_eq!(stored().await, Some(3));

    let _held = state.semaphore(id, 3).await.acquire_owned().await.unwrap();
    let crate::Json(own_view) =
        crate::channel_concurrency(State(state.clone()), own_headers.clone())
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_value(own_view).unwrap(),
        json!([{"channel_id": id, "in_use": 1, "limit": 3}])
    );
    let crate::Json(other_view) = crate::channel_concurrency(State(state.clone()), other_headers)
        .await
        .unwrap();
    assert!(other_view.is_empty());

    patch_channel(&state, &own_headers, id, json!({"concurrency_limit": null}))
        .await
        .unwrap();
    assert_eq!(stored().await, None);
    server.abort();
}

async fn channel(
    state: &AppState,
    account: Uuid,
    priority: i32,
    mode: &str,
    rules: Value,
    path: &str,
) -> Uuid {
    let id=sqlx::query_scalar("INSERT INTO channels(account_id,name,protocol,base_url,priority,model_mode,models,test_model) VALUES ($1,'fixture','openai',$2,$3,$4,$5,'fixture-model') RETURNING id")
        .bind(account).bind(format!("http://1.1.1.1{path}")).bind(priority).bind(mode).bind(rules).fetch_one(&state.db).await.unwrap();
    sqlx::query(
        "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,'fixture',$2)",
    )
    .bind(id)
    .bind(
        state
            .crypto
            .encrypt("mock-upstream-credential-only")
            .unwrap(),
    )
    .execute(&state.db)
    .await
    .unwrap();
    id
}

async fn key(db: &PgPool, account: Uuid, mode: &str, rules: Value) -> (Uuid, String) {
    let raw = format!("test-only-{}", Uuid::new_v4());
    let id=sqlx::query_scalar("INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models,model_mode) VALUES ($1,'fixture','test',$2,$3,$4) RETURNING id")
        .bind(account).bind(hash_token(&raw)).bind(rules).bind(mode).fetch_one(db).await.unwrap();
    (id, raw)
}

fn request(raw: &str, model: &str, stream: bool) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {raw}"))
        .body(Body::from(
            json!({"model":model,"messages":[],"stream":stream}).to_string(),
        ))
        .unwrap()
}

/// Signs in as the account's only user, creating it on first use.
async fn headers(db: &PgPool, account: Uuid, super_admin: bool) -> HeaderMap {
    let raw = format!("session-{}", Uuid::new_v4());
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM memberships WHERE account_id=$1")
            .bind(account)
            .fetch_optional(db)
            .await
            .unwrap();
    let user = match existing {
        Some(user) => {
            if super_admin {
                sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
                    .bind(user)
                    .execute(db)
                    .await
                    .unwrap();
            }
            user
        }
        None => {
            let user: Uuid = sqlx::query_scalar("INSERT INTO users(email,password_hash,is_super_admin) VALUES ($1,'test-only',$2) RETURNING id")
                .bind(format!("{}@example.invalid", Uuid::new_v4())).bind(super_admin).fetch_one(db).await.unwrap();
            sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
                .bind(account)
                .bind(user)
                .execute(db)
                .await
                .unwrap();
            user
        }
    };
    sqlx::query("INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
        .bind(user).bind(hash_token(&raw)).execute(db).await.unwrap();
    let mut result = HeaderMap::new();
    result.insert("cookie", format!("gateway_session={raw}").parse().unwrap());
    result
}

async fn priced(db: &PgPool, model: &str, input: f64) {
    sqlx::query("INSERT INTO model_catalog(pattern,label,input_per_1m,output_per_1m,cached_read_per_1m) VALUES ($1,$1,$2,8,0.5) ON CONFLICT(pattern) DO UPDATE SET input_per_1m=$2,output_per_1m=8,cached_read_per_1m=0.5")
        .bind(model).bind(input).execute(db).await.unwrap();
}

async fn patch_channel(
    state: &AppState,
    headers: &HeaderMap,
    channel: Uuid,
    input: Value,
) -> Result<()> {
    crate::update_channel(
        State(state.clone()),
        axum::extract::Path(channel),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .map(|_| ())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn channel_checks_report_missing_credentials_without_hiding_existing_channels(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let id = channel(&state, own, 10, "allowlist", json!(["fixture-model"]), "").await;
    sqlx::query("INSERT INTO channel_health(channel_id,status) VALUES ($1,'healthy')")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE channel_keys SET enabled=false WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    for result in [
        probe_channel(&state, own, id).await.map(|_| ()),
        fetch_channel_models(&state, own, id).await.map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(AppError::BadRequest(message)) if message.contains("未配置可用的上游 API Key"))
        );
    }
    sqlx::query("DELETE FROM channel_keys WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    for result in [
        probe_channel(&state, own, id).await.map(|_| ()),
        fetch_channel_models(&state, own, id).await.map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(AppError::BadRequest(message)) if message.contains("未配置可用的上游 API Key"))
        );
    }
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    for result in [
        probe_channel(&state, own, id).await.map(|_| ()),
        fetch_channel_models(&state, own, id).await.map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(AppError::BadRequest(message)) if message.contains("渠道已停用"))
        );
    }
    for channel_id in [id, Uuid::new_v4()] {
        assert!(matches!(
            probe_channel(&state, other, channel_id).await,
            Err(AppError::NotFound)
        ));
        assert!(matches!(
            fetch_channel_models(&state, other, channel_id).await,
            Err(AppError::NotFound)
        ));
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "healthy"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM channel_monitor_runs WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn channel_credentials_can_be_restored_and_rotated_without_leaking_or_crossing_accounts(
    db: PgPool,
) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let id = channel(&state, own, 10, "allowlist", json!(["fixture-model"]), "").await;
    let own_headers = headers(&db, own, false).await;
    let other_headers = headers(&db, other, false).await;
    sqlx::query("DELETE FROM channel_keys WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let restored = "mock-restored-credential-only";
    patch_channel(&state, &own_headers, id, json!({"secret":restored}))
        .await
        .unwrap();
    let cipher: String =
        sqlx::query_scalar("SELECT secret_ciphertext FROM channel_keys WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_ne!(cipher, restored);
    assert_eq!(state.crypto.decrypt(&cipher).unwrap(), restored);
    assert!(
        crate::check_channel(
            State(state.clone()),
            axum::extract::Path(id),
            own_headers.clone()
        )
        .await
        .unwrap()
        .0["healthy"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        fetch_channel_models(&state, own, id).await.unwrap(),
        ["discovered-private"]
    );
    patch_channel(&state, &own_headers, id, json!({"name":"renamed"}))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT secret_ciphertext FROM channel_keys WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap(),
        cipher
    );
    for invalid in [" ".to_owned(), "a".repeat(4097)] {
        assert!(matches!(
            patch_channel(
                &state,
                &own_headers,
                id,
                json!({"name":"must-not-change","secret":invalid})
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert!(matches!(
        patch_channel(
            &state,
            &other_headers,
            id,
            json!({"secret":"mock-foreign-credential"})
        )
        .await,
        Err(AppError::NotFound)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT name FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "renamed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT secret_ciphertext FROM channel_keys WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap(),
        cipher
    );
    sqlx::query("UPDATE channel_keys SET enabled=false WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let rotated = "mock-rotated-credential-only";
    patch_channel(&state, &own_headers, id, json!({"secret":rotated}))
        .await
        .unwrap();
    let keys: Vec<(String, bool)> =
        sqlx::query_as("SELECT secret_ciphertext,enabled FROM channel_keys WHERE channel_id=$1")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].1);
    assert_eq!(state.crypto.decrypt(&keys[0].0).unwrap(), rotated);
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT enabled FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "unknown"
    );
    patch_channel(&state, &own_headers, id, json!({"enabled":true}))
        .await
        .unwrap();
    assert!(probe_channel(&state, own, id).await.unwrap().0);
    sqlx::query(
        "UPDATE channel_keys SET secret_ciphertext='invalid-test-ciphertext' WHERE channel_id=$1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    assert!(
        matches!(probe_channel(&state, own, id).await, Err(AppError::BadRequest(message)) if message.contains("上游 API Key 无法读取") && !message.contains("invalid-test-ciphertext"))
    );
    let listed = crate::list_channels(State(state.clone()), own_headers.clone())
        .await
        .unwrap();
    let audit =
        sqlx::query_scalar::<_, Value>("SELECT metadata FROM audit_logs WHERE account_id=$1")
            .bind(own)
            .fetch_all(&db)
            .await
            .unwrap();
    for output in [
        serde_json::to_string(&listed.0).unwrap(),
        serde_json::to_string(&audit).unwrap(),
    ] {
        assert!(
            !output.contains(restored) && !output.contains(rotated) && !output.contains(&cipher)
        );
    }
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn routing_priority_catalog_isolation_and_key_revocation(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let prefix = channel(&state, own, 10, "allowlist", json!(["test-*"]), "").await;
    let exact = channel(&state, own, 20, "allowlist", json!(["test-model"]), "").await;
    let pass = channel(&state, own, 30, "passthrough", json!([]), "").await;
    channel(&state, other, 0, "passthrough", json!([]), "").await;
    priced(&db, "test-model", 2.0).await;
    // Legacy account entries must not expand the system catalog.
    for account in [own, other] {
        sqlx::query("INSERT INTO account_models(account_id,model,label,protocol,status) VALUES ($1,'test-private','Legacy','openai','registered')")
            .bind(account).execute(&db).await.unwrap();
    }
    let (id, raw) = key(&db, own, "allowlist", json!(["test-*"])).await;
    let candidates = load_candidates(&state, own, "openai", "test-model", None)
        .await
        .unwrap();
    assert_eq!(
        candidates.iter().map(|c| c.channel_id).collect::<Vec<_>>(),
        [prefix, exact, pass]
    );
    let mut auth = HeaderMap::new();
    auth.insert("authorization", format!("Bearer {raw}").parse().unwrap());
    let authenticated = authenticate_virtual_key(&state, &auth).await.unwrap();
    let response = model_catalog(&state, &authenticated).await.unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], "test-model");
    assert!(matches!(
        handle_inner(state.clone(), request(&raw, "other-model", false)).await,
        Err(AppError::Coded(_, "model_not_allowed", _))
    ));
    // Wildcard requests are invalid regardless of the key's permissions.
    assert!(matches!(
        handle_inner(state.clone(), request(&raw, "other-*", false)).await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(prefix)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        load_candidates(&state, own, "openai", "test-model", None)
            .await
            .unwrap()[0]
            .channel_id,
        exact
    );
    // A suspended account's keys stop authenticating until it is restored.
    sqlx::query("UPDATE accounts SET disabled_at=now() WHERE id=$1")
        .bind(own)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        authenticate_virtual_key(&state, &auth).await,
        Err(AppError::Coded(_, "key_paused", _))
    ));
    sqlx::query("UPDATE accounts SET disabled_at=NULL WHERE id=$1")
        .bind(own)
        .execute(&db)
        .await
        .unwrap();
    authenticate_virtual_key(&state, &auth).await.unwrap();
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        authenticate_virtual_key(&state, &auth).await,
        Err(AppError::Coded(_, "key_paused", _))
    ));
    sqlx::query("UPDATE virtual_api_keys SET enabled=true,revoked_at=now() WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        authenticate_virtual_key(&state, &auth).await,
        Err(AppError::Coded(_, "invalid_api_key", _))
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn mapped_upstream_exact_price_is_snapshotted_and_unknown_models_still_work(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let channel = channel(&state, own, 10, "passthrough", json!([]), "").await;
    sqlx::query("UPDATE channels SET model_mapping=$2 WHERE id=$1")
        .bind(channel)
        .bind(json!({"team-chat":"priced-concrete"}))
        .execute(&db)
        .await
        .unwrap();
    priced(&db, "priced-concrete", 2.0).await;
    priced(&db, "team-chat", 900.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let response = handle_inner(state.clone(), request(&raw, "team-chat", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let row=sqlx::query("SELECT id,model,upstream_model,cost_usd,price_snapshot FROM usage_events WHERE virtual_key_id=$1")
        .bind(id).fetch_one(&db).await.unwrap();
    let event: Uuid = row.get("id");
    assert_eq!(row.get::<String, _>("model"), "team-chat");
    assert_eq!(row.get::<String, _>("upstream_model"), "priced-concrete");
    assert!((row.get::<f64, _>("cost_usd") - 2.5).abs() < 1e-9);
    priced(&db, "priced-concrete", 200.0).await;
    update_usage_event(
        state.clone(),
        event,
        own,
        None,
        100,
        Some(TokenUsage {
            input: 1_000_000,
            output: 100_000,
            cached: Some(200_000),
        }),
    )
    .await;
    let cost: f64 = sqlx::query_scalar("SELECT cost_usd FROM usage_events WHERE id=$1")
        .bind(event)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!((cost - 2.5).abs() < 1e-9);
    assert_eq!(
        handle_inner(
            state.clone(),
            request(&raw, "unlisted-private-model", false)
        )
        .await
        .unwrap()
        .status(),
        200
    );
    assert!(
        !models::routable_catalog(&db, own)
            .await
            .unwrap()
            .contains_key("unlisted-private-model")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM account_models WHERE account_id=$1")
            .bind(own)
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM usage_events WHERE virtual_key_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn unsupported_model_fails_over_but_parameter_errors_do_not(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let first = channel(
        &state,
        own,
        10,
        "allowlist",
        json!(["fallback-*"]),
        "/missing",
    )
    .await;
    let second = channel(&state, own, 20, "allowlist", json!(["fallback-model"]), "").await;
    priced(&db, "fallback-model", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "fallback-model", false))
            .await
            .unwrap()
            .status(),
        200
    );
    let actual:Uuid=sqlx::query_scalar("SELECT channel_id FROM usage_events WHERE virtual_key_id=$1 ORDER BY created_at DESC LIMIT 1").bind(id).fetch_one(&db).await.unwrap();
    assert_eq!(actual, second);
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/bad' WHERE id=$1")
        .bind(first)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "fallback-model", false))
            .await
            .unwrap()
            .status(),
        400
    );
    server.abort();
}

/// Waits for a background availability update to reach `expected`.
async fn wait_health(db: &PgPool, channel: Uuid, expected: (&str, i32)) {
    let mut actual = None;
    for _ in 0..100 {
        actual = sqlx::query_as::<_, (String, i32)>(
            "SELECT status,consecutive_failures FROM channel_health WHERE channel_id=$1",
        )
        .bind(channel)
        .fetch_optional(db)
        .await
        .unwrap();
        if actual
            .as_ref()
            .is_some_and(|(status, failures)| (status.as_str(), *failures) == expected)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("channel health {actual:?}, expected {expected:?}");
}

async fn last_channel(db: &PgPool, key: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT channel_id FROM usage_events WHERE virtual_key_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(key)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn real_failures_reach_the_threshold_and_unavailable_channels_remain_a_last_resort(
    db: PgPool,
) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let first = channel(&state, own, 10, "allowlist", json!(["fo-model"]), "/fail").await;
    let second = channel(&state, own, 20, "allowlist", json!(["fo-model"]), "").await;
    priced(&db, "fo-model", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let send = || async {
        let response = handle_inner(state.clone(), request(&raw, "fo-model", false))
            .await
            .unwrap();
        let status = response.status();
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        status
    };
    let failed_calls = || async {
        calls
            .lock()
            .await
            .iter()
            .filter(|call| call["url"].as_str().unwrap().contains("/fail/"))
            .count()
    };
    // A 5xx fails over and counts once; the default threshold is two.
    assert_eq!(send().await, 200);
    wait_health(&db, first, ("unknown", 1)).await;
    assert_eq!(send().await, 200);
    wait_health(&db, first, ("unhealthy", 2)).await;
    // Unavailable now: routed last, so the healthy channel answers alone.
    assert_eq!(send().await, 200);
    assert_eq!(failed_calls().await, 2);
    assert_eq!(last_channel(&db, id).await, second);

    // With no other channel left it is still tried, and a success recovers it.
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(second)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1' WHERE id=$1")
        .bind(first)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send().await, 200);
    assert_eq!(last_channel(&db, id).await, first);
    wait_health(&db, first, ("healthy", 0)).await;

    // Under the observe policy an unavailable channel keeps its priority.
    sqlx::query("UPDATE channels SET enabled=true WHERE id=$1")
        .bind(second)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE channel_health SET status='unhealthy',consecutive_failures=2 WHERE channel_id=$1",
    )
    .bind(first)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(send().await, 200);
    assert_eq!(last_channel(&db, id).await, second);
    sqlx::query(
        "UPDATE channel_health SET status='unhealthy',consecutive_failures=2 WHERE channel_id=$1",
    )
    .bind(first)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO channel_monitor_configs(channel_id,availability_route_policy) VALUES ($1,'observe')")
        .bind(first)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send().await, 200);
    assert_eq!(last_channel(&db, id).await, first);
    wait_health(&db, first, ("healthy", 0)).await;

    // Real requests do not count while availability is not monitored.
    sqlx::query(
        "UPDATE channel_monitor_configs SET availability_enabled=false WHERE channel_id=$1",
    )
    .bind(first)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/fail' WHERE id=$1")
        .bind(first)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send().await, 200);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    wait_health(&db, first, ("healthy", 0)).await;
    server.abort();
}

/// Seconds left on a channel model's cooldown, once the background write lands.
async fn cooldown_ttl(state: &AppState, channel: Uuid, model: &str) -> Option<i64> {
    let mut conn = state.redis_connection().await.unwrap();
    for _ in 0..50 {
        let ttl: i64 = redis::cmd("TTL")
            .arg(format!("meta-router:cooldown:{channel}:{model}"))
            .query_async(&mut conn)
            .await
            .unwrap();
        if ttl > 0 {
            return Some(ttl);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    None
}

async fn redis_del(state: &AppState, key: String) {
    let mut conn = state.redis_connection().await.unwrap();
    let _: i64 = redis::cmd("DEL")
        .arg(key)
        .query_async(&mut conn)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn rate_limited_channels_cool_down_and_double_until_they_answer(db: PgPool) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let limited = channel(
        &state,
        own,
        10,
        "allowlist",
        json!(["cd-model"]),
        "/limited",
    )
    .await;
    let fallback = channel(&state, own, 20, "allowlist", json!(["cd-model"]), "").await;
    priced(&db, "cd-model", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let send = |state: AppState| {
        let raw = raw.clone();
        async move {
            let response = handle_inner(state, request(&raw, "cd-model", false))
                .await
                .unwrap();
            let status = response.status();
            to_bytes(response.into_body(), usize::MAX).await.unwrap();
            status
        }
    };
    let limited_calls = || async {
        calls
            .lock()
            .await
            .iter()
            .filter(|call| call["url"].as_str().unwrap().contains("/limited"))
            .count()
    };
    let strikes = format!("meta-router:cooldown_strikes:{limited}:cd-model");
    let cooling = format!("meta-router:cooldown:{limited}:cd-model");

    // A rate limit fails over without counting and cools the channel for 30 seconds.
    assert_eq!(send(state.clone()).await, 200);
    assert_eq!(last_channel(&db, id).await, fallback);
    let ttl = cooldown_ttl(&state, limited, "cd-model").await.unwrap();
    assert!((1..=30).contains(&ttl), "{ttl}");
    let health: Option<i32> =
        sqlx::query_scalar("SELECT consecutive_failures FROM channel_health WHERE channel_id=$1")
            .bind(limited)
            .fetch_optional(&db)
            .await
            .unwrap();
    assert!(health.unwrap_or(0) == 0);
    // While cooling it is tried after the fallback, so it is not called.
    assert_eq!(send(state.clone()).await, 200);
    assert_eq!(limited_calls().await, 1);
    // Once the cooldown ends it is first again; another rate limit doubles it.
    redis_del(&state, cooling.clone()).await;
    assert_eq!(send(state.clone()).await, 200);
    assert_eq!(limited_calls().await, 2);
    let ttl = cooldown_ttl(&state, limited, "cd-model").await.unwrap();
    assert!((31..=60).contains(&ttl), "{ttl}");

    // Every channel cooling: still tried in order, and the rate limit reaches the client.
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(fallback)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send(state.clone()).await, 429);
    assert_eq!(limited_calls().await, 3);
    sqlx::query("UPDATE channels SET enabled=true WHERE id=$1")
        .bind(fallback)
        .execute(&db)
        .await
        .unwrap();

    // A success clears the count, so the next rate limit starts at 30 seconds again.
    redis_del(&state, cooling.clone()).await;
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1' WHERE id=$1")
        .bind(limited)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send(state.clone()).await, 200);
    assert_eq!(last_channel(&db, id).await, limited);
    let mut conn = state.redis_connection().await.unwrap();
    let mut cleared = false;
    for _ in 0..50 {
        let exists: i64 = redis::cmd("EXISTS")
            .arg(&strikes)
            .query_async(&mut conn)
            .await
            .unwrap();
        if exists == 0 {
            cleared = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(cleared);

    // Retry-After replaces the schedule unless turned off.
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/limited-retry' WHERE id=$1")
        .bind(limited)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(send(state.clone()).await, 200);
    let ttl = cooldown_ttl(&state, limited, "cd-model").await.unwrap();
    assert!((1..=7).contains(&ttl), "{ttl}");
    redis_del(&state, cooling.clone()).await;
    *state.routing_settings.write().unwrap() = crate::routing::RoutingSettings {
        honor_retry_after: false,
        ..Default::default()
    };
    assert_eq!(send(state.clone()).await, 200);
    let ttl = cooldown_ttl(&state, limited, "cd-model").await.unwrap();
    assert!((31..=60).contains(&ttl), "{ttl}");

    // Turned off: cooling channels are routed as before and nothing new is written.
    *state.routing_settings.write().unwrap() = crate::routing::RoutingSettings {
        cooldown_enabled: false,
        ..Default::default()
    };
    let before = limited_calls().await;
    assert_eq!(send(state.clone()).await, 200);
    assert_eq!(limited_calls().await, before + 1);
    redis_del(&state, cooling.clone()).await;
    assert_eq!(send(state.clone()).await, 200);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut conn = state.redis_connection().await.unwrap();
    let exists: i64 = redis::cmd("EXISTS")
        .arg(&cooling)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(exists, 0);

    // Without Redis requests route as before.
    *state.routing_settings.write().unwrap() = Default::default();
    let mut offline = state.clone();
    offline.redis = redis::Client::open("redis://127.0.0.1:1/").unwrap();
    offline.redis_connection = Default::default();
    let before = limited_calls().await;
    assert_eq!(send(offline).await, 200);
    assert_eq!(limited_calls().await, before + 1);
    assert_eq!(last_channel(&db, id).await, fallback);
    redis_del(&state, cooling).await;
    redis_del(&state, strikes).await;
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn refused_credentials_and_balances_fail_over_and_count_toward_availability(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    priced(&db, "rf-model", 2.0).await;
    for (path, kind) in [
        ("/denied", "credential"),
        ("/quota", "quota"),
        ("/credit", "quota"),
    ] {
        let own = account(&db).await;
        let refused = channel(&state, own, 10, "allowlist", json!(["rf-model"]), path).await;
        let fallback = channel(&state, own, 20, "allowlist", json!(["rf-model"]), "").await;
        let (id, raw) = key(&db, own, "all", json!([])).await;
        let send = || async {
            handle_inner(state.clone(), request(&raw, "rf-model", false))
                .await
                .unwrap()
                .status()
        };
        assert_eq!(send().await, 200, "{path}");
        assert_eq!(last_channel(&db, id).await, fallback, "{path}");
        wait_health(&db, refused, ("unknown", 1)).await;
        assert_eq!(send().await, 200, "{path}");
        wait_health(&db, refused, ("unhealthy", 2)).await;
        let (error_kind, alert_open): (Option<String>, bool) = sqlx::query_as(
            "SELECT last_error_kind,alert_open FROM channel_health WHERE channel_id=$1",
        )
        .bind(refused)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(error_kind.as_deref(), Some(kind), "{path}");
        assert!(alert_open, "{path}");
        assert_eq!(
            cooldown_ttl(&state, refused, "rf-model").await,
            None,
            "{path}"
        );
        // With no other channel the client sees the upstream's error as before.
        sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
            .bind(fallback)
            .execute(&db)
            .await
            .unwrap();
        let expected = match path {
            "/denied" => 401,
            "/quota" => 429,
            _ => 400,
        };
        assert_eq!(send().await, expected, "{path}");
    }
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn routing_settings_are_for_super_admins_validated_and_apply_at_once(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let member = headers(&db, account(&db).await, false).await;
    let admin = headers(&db, account(&db).await, true).await;
    let update = |headers: HeaderMap, input: Value| {
        crate::routing::update_settings(
            State(state.clone()),
            headers,
            Json(serde_json::from_value(input).unwrap()),
        )
    };
    assert!(
        crate::routing::get_settings(State(state.clone()), member.clone())
            .await
            .is_err()
    );
    assert!(
        update(member, json!({"cooldown_enabled": false}))
            .await
            .is_err()
    );
    for invalid in [29, 3601] {
        assert!(matches!(
            update(admin.clone(), json!({"max_cooldown_seconds": invalid})).await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert_eq!(state.routing_settings(), Default::default());
    let Json(saved) = update(
        admin.clone(),
        json!({"max_cooldown_seconds": 600, "honor_retry_after": false}),
    )
    .await
    .unwrap();
    let expected = crate::routing::RoutingSettings {
        max_cooldown_seconds: 600,
        honor_retry_after: false,
        ..Default::default()
    };
    assert_eq!(saved, expected);
    assert_eq!(state.routing_settings(), expected);
    let Json(read) = crate::routing::get_settings(State(state.clone()), admin)
        .await
        .unwrap();
    assert_eq!(read, expected);
    assert_eq!(crate::routing::load(&db).await, expected);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn streams_without_output_in_time_fail_over_except_on_the_last_channel(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let slow = channel(&state, own, 10, "allowlist", json!(["ft-model"]), "/slow").await;
    let fast = channel(&state, own, 20, "allowlist", json!(["ft-model"]), "").await;
    sqlx::query("UPDATE channels SET first_token_timeout_seconds=1 WHERE id=$1")
        .bind(slow)
        .execute(&db)
        .await
        .unwrap();
    priced(&db, "ft-model", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let stream = || async {
        let response = handle_inner(state.clone(), request(&raw, "ft-model", true))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    };
    let first_outcome = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT request_diagnostics->'attempts'->0->>'outcome' FROM usage_events
             WHERE virtual_key_id=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    // Headers arrive at once but output does not: nothing of it reaches the client.
    let body = stream().await;
    assert!(
        body.contains("Hi") && !body.contains("assistant") && !body.contains("late"),
        "{body}"
    );
    assert_eq!(last_channel(&db, id).await, fast);
    assert_eq!(first_outcome().await, "first_token_timeout");
    wait_health(&db, slow, ("unknown", 1)).await;
    // The same applies when even the headers are late.
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/slow-headers' WHERE id=$1")
        .bind(slow)
        .execute(&db)
        .await
        .unwrap();
    assert!(stream().await.contains("Hi"));
    assert_eq!(first_outcome().await, "first_token_timeout");
    wait_health(&db, slow, ("unhealthy", 2)).await;
    // The last channel has nowhere to fail over to, so it is waited for.
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(fast)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET base_url='http://1.1.1.1/slow' WHERE id=$1")
        .bind(slow)
        .execute(&db)
        .await
        .unwrap();
    let body = stream().await;
    assert!(
        body.contains("assistant") && body.contains("late"),
        "{body}"
    );
    assert_eq!(last_channel(&db, id).await, slow);
    wait_health(&db, slow, ("healthy", 0)).await;
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn availability_settings_are_validated_and_a_lower_threshold_applies_at_once(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let id = channel(&state, own, 10, "allowlist", json!(["any-model"]), "").await;
    sqlx::query("INSERT INTO channel_monitor_configs(channel_id) VALUES ($1)")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_health(channel_id,status,consecutive_failures) VALUES ($1,'healthy',1)")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let update = |account: Uuid, input: Value| {
        let state = state.clone();
        let db = db.clone();
        async move {
            let session = headers(&db, account, false).await;
            crate::update_channel_monitor(
                State(state),
                axum::extract::Path(id),
                session,
                Json(serde_json::from_value(input).unwrap()),
            )
            .await
        }
    };
    for invalid in [
        json!({"availability_failure_threshold": 0}),
        json!({"availability_failure_threshold": 6}),
        json!({"availability_route_policy": "remove"}),
        json!({"first_token_timeout_seconds": 4}),
        json!({"first_token_timeout_seconds": 301}),
    ] {
        assert!(matches!(
            update(own, invalid).await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert!(matches!(
        update(other, json!({"first_token_timeout_seconds": 90})).await,
        Err(AppError::NotFound)
    ));
    let Json(view) = update(
        own,
        json!({"availability_failure_threshold": 1, "availability_route_policy": "observe", "first_token_timeout_seconds": 90}),
    )
    .await
    .unwrap();
    assert_eq!(view.availability_failure_threshold, 1);
    assert_eq!(view.availability_route_policy, "observe");
    assert_eq!(view.first_token_timeout_seconds, 90);
    assert_eq!(view.availability_status, "unhealthy");
    assert_eq!(view.availability_failures, 1);
    // Counted failures bring the next check forward to ten minutes, then back
    // off by how long the channel has been failing.
    let Json(view) = update(own, json!({"availability_interval_seconds": 3600}))
        .await
        .unwrap();
    assert_eq!(view.availability_interval_seconds, 3600);
    let next_check = || async {
        sqlx::query_scalar::<_, f64>(
            "SELECT EXTRACT(EPOCH FROM next_availability_at-now())::float8 FROM channel_monitor_configs WHERE channel_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert!((550.0..=600.0).contains(&next_check().await));
    let failing_for = |minutes: i32| {
        let db = db.clone();
        async move {
            sqlx::query("UPDATE channel_health SET failing_since=now()-make_interval(mins => $2),alert_open=true WHERE channel_id=$1")
                .bind(id)
                .bind(minutes)
                .execute(&db)
                .await
                .unwrap();
        }
    };
    // A burst of failed requests does not jump to the regular interval.
    failing_for(20).await;
    for _ in 0..10 {
        update_health(&state, id, Health::Failure("test failure"), Source::Traffic)
            .await
            .unwrap();
    }
    assert!((1150.0..=1260.0).contains(&next_check().await));
    failing_for(120).await;
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert!((3550.0..=3600.0).contains(&next_check().await));
    // An unavailable channel without an alert is checked again when one is due.
    sqlx::query("UPDATE channel_health SET alert_open=false,unhealthy_since=now()-interval '25 minutes' WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert!((250.0..=300.0).contains(&next_check().await));
    // Once that time has passed, the backoff applies again.
    sqlx::query("UPDATE channel_health SET unhealthy_since=now()-interval '40 minutes',last_alert_at=now() WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert!(next_check().await > 3500.0);
    update_health(&state, id, Health::Success, Source::Traffic)
        .await
        .unwrap();
    assert!(next_check().await > 3500.0);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT failing_since IS NULL FROM channel_health WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    );
    sqlx::query("UPDATE channel_health SET last_alert_at=NULL WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    // Short outages stay silent; a refused credential opens one alert, which
    // recovery closes while the daily limit stays.
    let alert = || async {
        sqlx::query_as::<_, (bool, bool, bool)>(
            "SELECT unhealthy_since IS NOT NULL,alert_open,last_alert_at IS NOT NULL FROM channel_health WHERE channel_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert_eq!(alert().await, (true, false, false));
    update_health(
        &state,
        id,
        Health::Rejected("上游返回 HTTP 401", "credential"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(alert().await, (true, true, true));
    update_health(
        &state,
        id,
        Health::Unreached("source key paused"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(alert().await, (true, true, true));
    update_health(&state, id, Health::Success, Source::Check)
        .await
        .unwrap();
    assert_eq!(alert().await, (false, false, true));
    // A rate limit is shown but neither counted nor alerted, and keeps the
    // verdict; a refusal remembers what to fix until a success.
    let observation = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_observation FROM channel_health WHERE channel_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    let health = || async {
        sqlx::query_as::<_, (String, i32, Option<String>)>(
            "SELECT status,consecutive_failures,last_error_kind FROM channel_health WHERE channel_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    update_health(
        &state,
        id,
        Health::Uncounted("rate_limited", "上游限流"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(health().await, ("healthy".to_owned(), 0, None));
    assert_eq!(observation().await.as_deref(), Some("rate_limited"));
    update_health(
        &state,
        id,
        Health::Uncounted("model_unavailable", "测试模型不可用"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(health().await, ("healthy".to_owned(), 0, None));
    assert_eq!(observation().await.as_deref(), Some("model_unavailable"));
    assert_eq!(alert().await, (false, false, true));
    update_health(
        &state,
        id,
        Health::Rejected("上游返回 HTTP 402", "quota"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(health().await.2.as_deref(), Some("quota"));
    update_health(&state, id, Health::Success, Source::Check)
        .await
        .unwrap();
    assert_eq!(health().await, ("healthy".to_owned(), 0, None));
    // An outage that has lasted long enough alerts on the next check.
    sqlx::query("UPDATE channel_health SET last_alert_at=NULL WHERE channel_id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    update_health(&state, id, Health::Failure("test failure"), Source::Traffic)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE channel_health SET unhealthy_since=now()-interval '31 minutes' WHERE channel_id=$1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    update_health(&state, id, Health::Failure("test failure"), Source::Traffic)
        .await
        .unwrap();
    assert_eq!(alert().await, (true, false, false));
    update_health(&state, id, Health::Failure("test failure"), Source::Check)
        .await
        .unwrap();
    assert_eq!(alert().await, (true, true, true));
    // Checks that do not conclude keep an unavailable channel out of routing
    // instead of flipping it back and forth.
    let failures = health().await.1;
    update_health(
        &state,
        id,
        Health::Uncounted("rate_limited", "上游限流"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(health().await, ("unhealthy".to_owned(), failures, None));
    assert_eq!(observation().await.as_deref(), Some("rate_limited"));
    // The tip keeps showing what made it unavailable.
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_error FROM channel_health WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
        .as_deref(),
        Some("test failure")
    );
    // Without a counted failure for long, the verdict expires but the alert
    // stays open for the recovery email.
    sqlx::query(
        "UPDATE channel_health SET last_failure_at=now()-interval '1 day' WHERE channel_id=$1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    update_health(
        &state,
        id,
        Health::Unreached("source key paused"),
        Source::Check,
    )
    .await
    .unwrap();
    assert_eq!(health().await, ("unknown".to_owned(), 0, None));
    assert_eq!(alert().await, (false, true, true));
    update_health(&state, id, Health::Success, Source::Check)
        .await
        .unwrap();
    assert_eq!(alert().await, (false, false, true));
    assert_eq!(observation().await, None);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn synchronization_and_aliases_do_not_expand_the_system_catalog(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let id = channel(&state, own, 10, "allowlist", json!(["test-*"]), "").await;
    let headers = headers(&db, own, false).await;
    let _ = crate::sync_channel_models(
        State(state.clone()),
        axum::extract::Path(id),
        headers.clone(),
    )
    .await
    .unwrap();
    let row = sqlx::query("SELECT models,model_mode,discovered_models FROM channels WHERE id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(row.get::<Value, _>("models"), json!(["test-*"]));
    assert_eq!(row.get::<String, _>("model_mode"), "allowlist");
    assert_eq!(
        row.get::<Value, _>("discovered_models"),
        json!(["discovered-private"])
    );
    let input = serde_json::from_value(json!({"models":[]})).unwrap();
    assert!(matches!(
        crate::update_channel(
            State(state.clone()),
            axum::extract::Path(id),
            headers.clone(),
            Json(input)
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("UPDATE channels SET test_model=NULL WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_health(channel_id,status) VALUES ($1,'healthy')")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        probe_channel(&state, own, id).await,
        Err(AppError::BadRequest(message)) if message.contains("需要配置具体测试模型")
    ));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "healthy"
    );
    patch_channel(
        &state,
        &headers,
        id,
        json!({
            "models":["test-private"], "model_mapping":{"team-private":"test-private"}
        }),
    )
    .await
    .unwrap();
    let _ = crate::create_route(
        State(state.clone()),
        headers.clone(),
        Json(
            serde_json::from_value(json!({
                "channel_id":id, "public_model":"route-private", "upstream_model":"test-private",
                "protocol":"openai"
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    for name in ["test-private", "team-private", "route-private"] {
        let candidates = load_candidates(&state, own, "openai", name, None)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].channel_id, id);
        assert!(
            load_candidates(&state, other, "openai", name, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let (_, raw) = key(&db, own, "all", json!([])).await;
    let response = handle_inner(state.clone(), request(&raw, "team-private", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["model"], "test-private");
    let known = models::directory(&db).await.unwrap();
    for name in [
        "discovered-private",
        "test-private",
        "team-private",
        "route-private",
    ] {
        assert!(!known.iter().any(|item| item.model == name));
    }
    // Configured names, aliases and routes are listed; fetched names are not.
    let listed = ["route-private", "team-private", "test-private"];
    assert_eq!(
        models::routable_catalog(&db, own).await.unwrap(),
        listed
            .iter()
            .map(|name| (name.to_string(), "openai".to_string()))
            .collect::<std::collections::BTreeMap<_, _>>()
    );
    assert!(
        models::routable_catalog(&db, other)
            .await
            .unwrap()
            .is_empty()
    );
    let Json(channels) = crate::list_channels(State(state.clone()), headers.clone())
        .await
        .unwrap();
    assert_eq!(channels[0].known_models, listed);
    let Json(keys) = crate::list_keys(State(state.clone()), headers.clone())
        .await
        .unwrap();
    assert_eq!(keys[0].known_models, listed);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM account_models WHERE account_id=$1")
            .bind(own)
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let foreign_headers = self::headers(&db, other, false).await;
    assert!(matches!(
        crate::sync_channel_models(
            State(state.clone()),
            axum::extract::Path(id),
            foreign_headers
        )
        .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        crate::create_model_catalog(
            State(state),
            headers,
            Json(serde_json::from_value(json!({"pattern":"not-authorized","label":"no"})).unwrap())
        )
        .await,
        Err(AppError::Forbidden)
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn stream_usage_uses_immutable_exact_price_and_does_not_regress(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    priced(&db, "stream-model", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let response = handle_inner(state.clone(), request(&raw, "stream-model", true))
        .await
        .unwrap();
    priced(&db, "stream-model", 500.0).await;
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let event: Uuid = sqlx::query_scalar("SELECT id FROM usage_events WHERE virtual_key_id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    // Real traffic records what output performance and passive metering need.
    let timing = sqlx::query(
        "SELECT stream,first_token_ms,input_ascii_chars,input_other_chars FROM usage_events WHERE id=$1",
    )
    .bind(event)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(timing.get::<Option<bool>, _>("stream"), Some(true));
    assert!(timing.get::<Option<i32>, _>("first_token_ms").is_some());
    // The request has no messages: counted, as zero.
    assert_eq!(timing.get::<Option<i32>, _>("input_ascii_chars"), Some(0));
    assert_eq!(timing.get::<Option<i32>, _>("input_other_chars"), Some(0));
    // Deterministic finalization also tests out-of-order partial stream updates.
    update_usage_event(
        state.clone(),
        event,
        own,
        Some(1),
        100,
        Some(TokenUsage {
            input: 1_000_000,
            output: 100_000,
            cached: Some(200_000),
        }),
    )
    .await;
    update_usage_event(
        state.clone(),
        event,
        own,
        Some(1),
        20,
        Some(TokenUsage {
            input: 100,
            output: 1,
            cached: None,
        }),
    )
    .await;
    let row =
        sqlx::query("SELECT cost_usd,pricing_status,output_tokens FROM usage_events WHERE id=$1")
            .bind(event)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!((row.get::<f64, _>("cost_usd") - 2.5).abs() < 1e-9);
    assert_eq!(row.get::<String, _>("pricing_status"), "priced");
    assert_eq!(row.get::<i64, _>("output_tokens"), 100_000);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn catalog_management_counts_and_usage_summary_share_concrete_identities(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, true).await;
    let catalog_input = |model: &str, price: f64| {
        serde_json::from_value(json!({
        "pattern": model, "label": model, "provider": "Fixture", "protocols": ["openai"],
        "price": {"pattern": model, "label": model, "input_per_1m": price, "output_per_1m": 8, "cached_read_per_1m": 0.5}
    })).unwrap()
    };
    assert!(matches!(
        crate::create_model_catalog(
            State(state.clone()),
            session.clone(),
            Json(catalog_input("summary-*", 2.0))
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    // The catalog no longer manages prefix templates, priced or not.
    assert!(matches!(
        crate::create_model_catalog(
            State(state.clone()),
            session.clone(),
            Json(
                serde_json::from_value(
                    json!({"pattern":"summary-*","label":"summary-*","protocols":["openai"]})
                )
                .unwrap()
            )
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES ('legacy-template-*','legacy')")
        .execute(&db)
        .await
        .unwrap();
    let Json(listed) = crate::list_model_catalog(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert!(listed.iter().all(|item| !item.pattern.contains('*')));
    let Json(created) = crate::create_model_catalog(
        State(state.clone()),
        session.clone(),
        Json(catalog_input("summary-model", 2.0)),
    )
    .await
    .unwrap();
    channel(&state, own, 10, "allowlist", json!(["summary-*"]), "").await;
    let (_, raw) = key(&db, own, "allowlist", json!(["summary-*"])).await;
    let Json(channels) = crate::list_channels(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(channels[0].model_count, 1);
    assert_eq!(channels[0].known_models, ["summary-model"]);
    let Json(keys) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(keys[0].known_models, ["summary-model"]);
    let Json(catalog) = crate::list_model_catalog(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert!(
        catalog
            .iter()
            .any(|item| item.id == created.id && item.input_per_1m == Some(2.0))
    );
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "summary-model", false))
            .await
            .unwrap()
            .status(),
        200
    );
    let _ = crate::update_model_catalog(
        State(state.clone()),
        axum::extract::Path(created.id),
        session.clone(),
        Json(catalog_input("summary-model", 999.0)),
    )
    .await
    .unwrap();
    let query = || axum::extract::Query(serde_json::from_value(json!({"range":"24h"})).unwrap());
    let Json(summary) = crate::usage::summary(State(state.clone()), session.clone(), query())
        .await
        .unwrap();
    assert_eq!(summary["totals"]["requests"], 1);
    assert_eq!(summary["models"][0]["estimated_cost_usd"], 2.5);
    assert_eq!(summary["channels"][0]["estimated_cost_usd"], 2.5);
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "summary-unpriced", false))
            .await
            .unwrap()
            .status(),
        200
    );
    let Json(summary) = crate::usage::summary(State(state.clone()), session.clone(), query())
        .await
        .unwrap();
    assert_eq!(summary["totals"]["requests"], 2);
    // Requests without a cost add nothing and are not reported to the user.
    assert_eq!(summary["channels"][0]["estimated_cost_usd"], 2.5);
    assert!(
        summary["channels"][0]
            .get("unknown_cost_requests")
            .is_none()
    );
    let Json(events) = crate::usage::events(State(state.clone()), session.clone(), query())
        .await
        .unwrap();
    assert_eq!(events["total"], 2);
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["upstream_model"] == "summary-model" && event["cost_usd"] == 2.5)
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn channel_cost_details_follow_filters_and_preserve_known_amounts(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let other = account(&db).await;
    let session = headers(&db, own, false).await;
    let primary = channel(&state, own, 10, "passthrough", json!([]), "").await;
    let secondary = channel(&state, own, 20, "passthrough", json!([]), "").await;
    let foreign = channel(&state, other, 10, "passthrough", json!([]), "").await;
    for (account_id, channel_id, model, upstream, protocol, status, pricing_status, cost, age) in [
        (
            own,
            primary,
            "paid",
            Some("up-paid"),
            "openai",
            200,
            "priced",
            Some(2.5),
            0,
        ),
        (
            own,
            primary,
            "free",
            Some("up-free"),
            "openai",
            200,
            "priced",
            Some(0.0),
            0,
        ),
        (
            own,
            primary,
            "alias-a",
            Some("up-unpriced"),
            "openai",
            200,
            "unpriced",
            None,
            0,
        ),
        (
            own,
            primary,
            "alias-b",
            Some("up-unpriced"),
            "openai",
            200,
            "unpriced",
            None,
            0,
        ),
        (
            own,
            primary,
            "no-usage",
            Some("up-unpriced"),
            "openai",
            502,
            "pending_usage",
            None,
            0,
        ),
        (
            own,
            primary,
            "historical",
            None,
            "openai",
            200,
            "legacy",
            None,
            0,
        ),
        (
            own,
            primary,
            "anthropic-model",
            Some("up-anthropic"),
            "anthropic",
            200,
            "unpriced",
            None,
            0,
        ),
        (
            own,
            primary,
            "expired",
            Some("up-expired"),
            "openai",
            200,
            "unpriced",
            None,
            2,
        ),
        (
            own,
            secondary,
            "other-channel",
            None,
            "openai",
            200,
            "legacy",
            None,
            0,
        ),
        (
            other,
            foreign,
            "other-account",
            None,
            "openai",
            200,
            "legacy",
            None,
            0,
        ),
    ] {
        sqlx::query("INSERT INTO usage_events(account_id,channel_id,model,upstream_model,protocol,status,duration_ms,pricing_status,cost_usd,created_at) VALUES ($1,$2,$3,$4,$5,$6,10,$7,$8,now()-make_interval(days=>$9))")
            .bind(account_id).bind(channel_id).bind(model).bind(upstream).bind(protocol).bind(status)
            .bind(pricing_status).bind(cost).bind(age).execute(&db).await.unwrap();
    }
    let query = |extra: Value| {
        let mut input = json!({"range":"24h", "channel_id":primary});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        axum::extract::Query(serde_json::from_value(input).unwrap())
    };
    let Json(summary) =
        crate::usage::summary(State(state.clone()), session.clone(), query(json!({})))
            .await
            .unwrap();
    assert_eq!(summary["channels"].as_array().unwrap().len(), 1);
    let row = &summary["channels"][0];
    assert_eq!(row["estimated_cost_usd"], 2.5);
    for (filter, amount) in [
        (json!({"model":"free"}), json!(0.0)),
        (json!({"model":"alias-a"}), Value::Null),
        (json!({"protocol":"anthropic"}), Value::Null),
        (json!({"status":"error"}), Value::Null),
        (json!({"search":"paid"}), json!(2.5)),
        (json!({"channel_id":secondary}), Value::Null),
    ] {
        let Json(summary) =
            crate::usage::summary(State(state.clone()), session.clone(), query(filter))
                .await
                .unwrap();
        assert_eq!(summary["channels"][0]["estimated_cost_usd"], amount);
    }
    let Json(summary) = crate::usage::summary(
        State(state.clone()),
        session,
        query(json!({"channel_id":foreign})),
    )
    .await
    .unwrap();
    assert_eq!(summary["channels"], json!([]));
    server.abort();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn migration_preserves_permissions_and_archives_ambiguous_prices(db: PgPool) {
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|migration| migration.version < 33) {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let own = account(&db).await;
    sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES ('migration-known','Known'),('migration-*','Template')").execute(&db).await.unwrap();
    let prices = json!([
        {"pattern":"migration-known","label":"Known","input_per_1m":2,"output_per_1m":8,"cached_read_per_1m":0.5},
        {"pattern":"migration-unknown","label":"Unknown","input_per_1m":3,"output_per_1m":9,"cached_read_per_1m":0},
        {"pattern":"migration-*","label":"Prefix","input_per_1m":4,"output_per_1m":10,"cached_read_per_1m":0}
    ]).to_string();
    sqlx::query("INSERT INTO system_settings(key,value_ciphertext) VALUES ('model_pricing',$1) ON CONFLICT(key) DO UPDATE SET value_ciphertext=$1").bind(&prices).execute(&db).await.unwrap();
    for rules in [json!([]), json!(["migration-*"])] {
        sqlx::query("INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models) VALUES ($1,'fixture','test',$2,$3)").bind(own).bind(Uuid::new_v4().to_string()).bind(rules).execute(&db).await.unwrap();
    }
    sqlx::query("INSERT INTO channels(account_id,name,protocol,base_url,priority,models,model_mode) VALUES ($1,'fixture','openai','https://example.invalid',17,'[\"migration-private\"]','allowlist')").bind(own).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO usage_events(account_id,model,protocol,status,duration_ms,input_tokens,output_tokens) VALUES ($1,'migration-known','openai',200,10,100,10)").bind(own).execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0033_unified_model_management.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    let migrated = pricing::load(&db).await.unwrap();
    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].pattern, "migration-known");
    assert_eq!(migrated[0].input_per_1m, 2.0);
    let archived: String = sqlx::query_scalar(
        "SELECT value_ciphertext FROM system_settings WHERE key='model_pricing'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(archived, prices);
    let modes: Vec<String> = sqlx::query_scalar("SELECT model_mode FROM virtual_api_keys WHERE account_id=$1 ORDER BY jsonb_array_length(allowed_models)").bind(own).fetch_all(&db).await.unwrap();
    assert_eq!(modes, ["all", "allowlist"]);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT priority FROM channels WHERE account_id=$1")
            .bind(own)
            .fetch_one(&db)
            .await
            .unwrap(),
        17
    );
    assert!(
        !models::directory(&db)
            .await
            .unwrap()
            .iter()
            .any(|name| name.model == "migration-private")
    );
    let legacy: (String, Option<f64>, Option<String>, Option<Value>) = sqlx::query_as("SELECT pricing_status,cost_usd,upstream_model,price_snapshot FROM usage_events WHERE account_id=$1").bind(own).fetch_one(&db).await.unwrap();
    assert_eq!(legacy, ("legacy".into(), None, None, None));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn directory_matches_model_scoped_quality_routing(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let channel_id = channel(&state, own, 10, "allowlist", json!(["quality-*"]), "").await;
    for model in ["quality-one", "quality-two"] {
        priced(&db, model, 2.0).await;
    }
    let (id, _) = key(&db, own, "all", json!([])).await;
    let monitor: Uuid = sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,route_policy,status,valid_until) VALUES ($1,'quality-one','exclude','degraded',now()+interval '1 hour') RETURNING id")
        .bind(channel_id).fetch_one(&db).await.unwrap();
    assert!(
        load_candidates(&state, own, "openai", "quality-one", None)
            .await
            .unwrap()
            .is_empty()
    );
    let catalog = models::routable_catalog(&db, own).await.unwrap();
    assert!(!catalog.contains_key("quality-one"));
    assert!(catalog.contains_key("quality-two"));
    let session = headers(&db, own, false).await;
    let Json(keys) = crate::list_keys(State(state.clone()), session)
        .await
        .unwrap();
    assert_eq!(
        keys.iter().find(|key| key.id == id).unwrap().known_models,
        ["quality-two"]
    );
    for policy in ["observe", "deprioritize"] {
        sqlx::query("UPDATE quality_monitors SET route_policy=$2 WHERE id=$1")
            .bind(monitor)
            .bind(policy)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(
            load_candidates(&state, own, "openai", "quality-one", None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            models::routable_catalog(&db, own)
                .await
                .unwrap()
                .contains_key("quality-one")
        );
    }
    sqlx::query("UPDATE quality_monitors SET route_policy='exclude',valid_until=now()-interval '1 second' WHERE id=$1").bind(monitor).execute(&db).await.unwrap();
    assert_eq!(
        load_candidates(&state, own, "openai", "quality-one", None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        models::routable_catalog(&db, own)
            .await
            .unwrap()
            .contains_key("quality-one")
    );
    server.abort();
}

async fn preview_check(state: &AppState, headers: &HeaderMap, input: Value) -> Result<Value> {
    crate::preview_channel_check(
        State(state.clone()),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .map(|Json(value)| value)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn draft_previews_use_current_parameters_and_never_save_or_cross_accounts(db: PgPool) {
    let (state, server, requests) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let id = channel(
        &state,
        own,
        10,
        "allowlist",
        json!(["fixture-model"]),
        "/saved",
    )
    .await;
    let session = headers(&db, own, false).await;
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM channels c WHERE id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    let draft = json!({"channel_id":id,"protocol":"openai","base_url":"http://1.1.1.1/draft","model":"draft-model"});
    let result = preview_check(&state, &session, draft.clone())
        .await
        .unwrap();
    assert_eq!(result["healthy"], true);
    assert!(result["check_receipt"].is_string());
    assert!(!result.to_string().contains("mock-upstream-credential-only"));
    let first = requests.lock().await[0].clone();
    assert!(first["url"].as_str().unwrap().contains("/draft/"));
    assert_eq!(first["model"], "draft-model");
    assert_eq!(
        first["authorization"],
        "Bearer mock-upstream-credential-only"
    );
    let mut replacement = draft.clone();
    replacement["secret"] = json!("mock-replacement-only");
    let _ = crate::preview_channel_models(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(replacement.clone()).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        requests.lock().await[1]["authorization"],
        "Bearer mock-replacement-only"
    );
    assert!(
        requests.lock().await[1]["url"]
            .as_str()
            .unwrap()
            .contains("/draft/")
    );
    preview_check(&state, &session, replacement.clone())
        .await
        .unwrap();
    assert_eq!(
        requests.lock().await[2]["authorization"],
        "Bearer mock-replacement-only"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT to_jsonb(c) FROM channels c WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let other = account(&db).await;
    let foreign_session = headers(&db, other, false).await;
    for input in [draft, replacement] {
        assert!(matches!(
            preview_check(&state, &foreign_session, input.clone()).await,
            Err(AppError::NotFound)
        ));
        assert!(matches!(
            crate::preview_channel_models(
                State(state.clone()),
                foreign_session.clone(),
                Json(serde_json::from_value(input.clone()).unwrap())
            )
            .await,
            Err(AppError::NotFound)
        ));
    }
    assert_eq!(requests.lock().await.len(), 3);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn channel_test_model_null_clears_while_omission_preserves_and_required_modes_reject_it(
    db: PgPool,
) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let id = channel(&state, own, 10, "allowlist", json!(["fixture-model"]), "").await;
    patch_channel(&state, &session, id, json!({"name":"renamed"}))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>("SELECT test_model FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
            .as_deref(),
        Some("fixture-model")
    );
    patch_channel(&state, &session, id, json!({"test_model":null}))
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT test_model FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(probe_channel(&state, own, id).await.unwrap().0);
    patch_channel(&state, &session, id, json!({"test_model":"other-model"}))
        .await
        .unwrap();
    let _ = crate::update_channel_models(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        Json(
            serde_json::from_value(json!({"models":["fixture-model"],"test_model":null})).unwrap(),
        ),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT test_model FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
            .is_none()
    );
    for change in [
        json!({"model_mode":"passthrough","test_model":null}),
        json!({"models":["fixture-*"],"test_model":null}),
    ] {
        assert!(matches!(
            patch_channel(&state, &session, id, change).await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT model_mode FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "allowlist"
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn verified_preview_is_saved_without_another_upstream_request_and_rejects_changed_config(
    db: PgPool,
) {
    let (state, server, requests) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let draft = json!({"protocol":"openai","base_url":"http://1.1.1.1","model":"fixture-model","secret":"mock-only-new-key"});
    let checked = preview_check(&state, &session, draft).await.unwrap();
    let input = json!({"name":"new fixture","protocol":"openai","base_url":"http://1.1.1.1","models":["fixture-model"],"test_model":null,"secret":"mock-only-new-key","check_receipt":checked["check_receipt"]});
    for (field, value) in [
        ("base_url", json!("http://1.1.1.1/changed")),
        ("secret", json!("different-mock-key")),
        ("test_model", json!("different-model")),
        ("check_receipt", json!("tampered")),
    ] {
        let mut changed = input.clone();
        changed[field] = value;
        assert!(matches!(
            crate::create_channel(
                State(state.clone()),
                session.clone(),
                Json(serde_json::from_value(changed).unwrap())
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    let mut expired_check: Value = serde_json::from_str(
        &state
            .crypto
            .decrypt(checked["check_receipt"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    expired_check["checked_at"] = json!(Utc::now() - chrono::Duration::minutes(16));
    let mut expired = input.clone();
    expired["check_receipt"] = json!(state.crypto.encrypt(&expired_check.to_string()).unwrap());
    assert!(matches!(
        crate::create_channel(
            State(state.clone()),
            session.clone(),
            Json(serde_json::from_value(expired).unwrap())
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let other = account(&db).await;
    let foreign_session = headers(&db, other, false).await;
    assert!(matches!(
        crate::create_channel(
            State(state.clone()),
            foreign_session,
            Json(serde_json::from_value(input.clone()).unwrap())
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let Json(created) = crate::create_channel(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .unwrap();
    let id: Uuid = serde_json::from_value(created["id"].clone()).unwrap();
    assert_eq!(requests.lock().await.len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "healthy"
    );
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT next_availability_at > now() FROM channel_monitor_configs WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM channel_monitor_runs WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap(),
        1
    );
    sqlx::query(
        "UPDATE channels SET discovered_models='[\"old-only\"]',models_synced_at=now() WHERE id=$1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    let edit = json!({"channel_id":id,"protocol":"openai","base_url":"http://1.1.1.1/edited","model":"changed-model","secret":"mock-rotated-key"});
    let checked = preview_check(&state, &session, edit).await.unwrap();
    patch_channel(&state, &session, id, json!({"base_url":"http://1.1.1.1/edited","test_model":"changed-model","secret":"mock-rotated-key","check_receipt":checked["check_receipt"]})).await.unwrap();
    assert_eq!(requests.lock().await.len(), 2);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "healthy"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT discovered_models FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        json!([])
    );
    sqlx::query(
        "UPDATE channel_health SET status='unhealthy',checked_at=now() WHERE channel_id=$1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    patch_channel(
        &state,
        &session,
        id,
        json!({"name":"renamed after monitoring","check_receipt":checked["check_receipt"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "unhealthy"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM channel_monitor_runs WHERE channel_id=$1"
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap(),
        2
    );
    let before: String = sqlx::query_scalar("SELECT base_url FROM channels WHERE id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(
        patch_channel(
            &state,
            &session,
            id,
            json!({"base_url":"http://1.1.1.1/wrong","check_receipt":checked["check_receipt"]})
        )
        .await
        .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT base_url FROM channels WHERE id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap(),
        before
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn adopting_reference_prices_uses_catalog_and_preserves_concurrent_prices(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, true).await;
    let Json(entry) = crate::create_model_catalog(State(state.clone()), session.clone(), Json(serde_json::from_value(json!({"pattern":"reference-fixture","label":"Existing label","enabled":false,"protocols":["anthropic"]})).unwrap())).await.unwrap();
    let price = json!({"pattern":"reference-fixture","label":"Reference label","input_per_1m":2.0,"output_per_1m":8.0,"cached_read_per_1m":0.5,"synced_from":"openrouter"});
    let input = json!({"pattern":"reference-fixture","label":"Stale label","price":price,"only_unpriced":true});
    let Json(updated) = crate::update_model_catalog(
        State(state.clone()),
        axum::extract::Path(entry.id),
        session.clone(),
        Json(serde_json::from_value(input.clone()).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(updated.price_source.as_deref(), Some("openrouter"));
    assert_eq!(updated.input_per_1m, Some(2.0));
    assert_eq!(updated.label, "Existing label");
    assert!(!updated.enabled);
    assert_eq!(updated.protocols, ["anthropic"]);
    let mut conflict = input;
    conflict["price"]["input_per_1m"] = json!(9.0);
    assert!(matches!(
        crate::update_model_catalog(
            State(state.clone()),
            axum::extract::Path(entry.id),
            session,
            Json(serde_json::from_value(conflict).unwrap())
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, f64>("SELECT input_per_1m FROM model_catalog WHERE id=$1")
            .bind(entry.id)
            .fetch_one(&db)
            .await
            .unwrap(),
        2.0
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn saving_legacy_route_mappings_keeps_channel_health(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let id = channel(&state, own, 10, "allowlist", json!(["fixture-model"]), "").await;
    sqlx::query("INSERT INTO model_routes(account_id,channel_id,protocol,public_model,upstream_model) VALUES ($1,$2,'openai','legacy-alias','legacy-up')")
        .bind(own).bind(id).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO channel_health(channel_id,status,last_error) VALUES ($1,'unhealthy','HTTP 401')")
        .bind(id).execute(&db).await.unwrap();
    let health = || async {
        sqlx::query_scalar::<_, String>("SELECT status FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
    };
    // The editor sends legacy routes back as mappings with an unrelated rename.
    patch_channel(
        &state,
        &session,
        id,
        json!({"name":"renamed","model_mapping":{"legacy-alias":"legacy-up"}}),
    )
    .await
    .unwrap();
    assert_eq!(health().await, "unhealthy");
    patch_channel(
        &state,
        &session,
        id,
        json!({"model_mapping":{"legacy-alias":"legacy-next"}}),
    )
    .await
    .unwrap();
    assert_eq!(health().await, "unknown");
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn catalog_updates_keep_price_source_and_report_precise_errors(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, true).await;
    let update = |id: Uuid, input: Value| {
        crate::update_model_catalog(
            State(state.clone()),
            axum::extract::Path(id),
            session.clone(),
            Json(serde_json::from_value(input).unwrap()),
        )
    };
    let price = json!({"pattern":"source-fixture","label":"Source","input_per_1m":2.0,"output_per_1m":8.0,"cached_read_per_1m":0.5,"synced_from":"openrouter"});
    let Json(entry) = crate::create_model_catalog(
        State(state.clone()),
        session.clone(),
        Json(
            serde_json::from_value(
                json!({"pattern":"source-fixture","label":"Source","price":price}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let mut same = json!({"pattern":"source-fixture","label":"Renamed","price":price});
    same["price"]["synced_from"] = Value::Null;
    let Json(renamed) = update(entry.id, same.clone()).await.unwrap();
    assert_eq!(renamed.label, "Renamed");
    assert_eq!(renamed.price_source.as_deref(), Some("openrouter"));
    let mut changed = same.clone();
    changed["price"]["input_per_1m"] = json!(3.0);
    let Json(manual) = update(entry.id, changed).await.unwrap();
    assert_eq!(manual.price_source, None);
    let mut both = same.clone();
    both["clear_price"] = json!(true);
    assert!(matches!(
        update(entry.id, both).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        update(
            Uuid::new_v4(),
            json!({"pattern":"source-fixture","label":"Source","only_unpriced":true})
        )
        .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        update(entry.id, json!({"pattern":"source-renamed","label":"Source"})).await,
        Err(AppError::BadRequest(message)) if message.contains("不可修改")
    ));
    assert!(matches!(
        update(
            entry.id,
            json!({"pattern":"source-fixture","label":"Source","only_unpriced":true})
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn bili_tags_are_stripped_before_anthropic_upstreams_and_counted(db: PgPool) {
    let (state, server, requests) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let channel_id = channel(&state, own, 10, "passthrough", json!([]), "/anthropic").await;
    sqlx::query("UPDATE channels SET protocol='anthropic' WHERE id=$1 AND account_id=$2")
        .bind(channel_id)
        .bind(own)
        .execute(&db)
        .await
        .unwrap();
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    let tag = "<acp tokens=\"1K\" type=\"text\">m00001</acp>\n";
    let messages = json!([
        {"role": "user", "content": format!("{tag}question")},
        {"role": "assistant", "content": [{"type": "text", "text": format!("{tag}answer")}]},
        {"role": "user", "content": "next"},
    ]);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-api-key", &raw)
        .body(Body::from(
            json!({"model": "fixture", "max_tokens": 1, "messages": messages}).to_string(),
        ))
        .unwrap();
    let response = handle_inner(state.clone(), request).await.unwrap();
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let sent = requests.lock().await.last().unwrap()["messages"].clone();
    assert_eq!(sent[0]["content"], format!("{tag}question"));
    assert_eq!(sent[1]["content"][0]["text"], "answer");
    let timing: sqlx::types::Json<crate::request_diagnostics::Diagnostics> = sqlx::query_scalar(
        "SELECT request_diagnostics FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2",
    )
    .bind(own)
    .bind(key_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(timing.acp_stripped, Some(1));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn bili_tags_are_stripped_before_responses_upstreams_and_counted(db: PgPool) {
    let (state, server, requests) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    let tag = "<acp tokens=\"1K\" type=\"text\">m00001</acp>\n";
    let input = json!([
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": format!("{tag}question")}]},
        {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": format!("{tag}answer")}]},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "next"}]},
    ]);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", format!("Bearer {raw}"))
        .body(Body::from(
            json!({"model": "fixture", "input": input}).to_string(),
        ))
        .unwrap();
    let response = handle_inner(state.clone(), request).await.unwrap();
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let sent = requests.lock().await.last().unwrap()["input"].clone();
    assert_eq!(sent[0]["content"][0]["text"], format!("{tag}question"));
    assert_eq!(sent[1]["content"][0]["text"], "answer");
    let timing: sqlx::types::Json<crate::request_diagnostics::Diagnostics> = sqlx::query_scalar(
        "SELECT request_diagnostics FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2",
    )
    .bind(own)
    .bind(key_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(timing.acp_stripped, Some(1));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn gateway_prices_come_from_the_cache_and_follow_platform_edits_at_once(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    priced(&db, "cached-model", 2.0).await;
    pricing::set(&state, pricing::load(&db).await.unwrap());
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let latest_cost = || async {
        sqlx::query_scalar::<_, f64>(
            "SELECT cost_usd FROM usage_events WHERE virtual_key_id=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    // An edit behind the gateway's back waits for the next scheduled refresh.
    priced(&db, "cached-model", 200.0).await;
    let response = handle_inner(state.clone(), request(&raw, "cached-model", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!((latest_cost().await - 2.5).abs() < 1e-9);
    // An edit through the platform API is used by the very next request.
    let entry: Uuid =
        sqlx::query_scalar("SELECT id FROM model_catalog WHERE pattern='cached-model'")
            .fetch_one(&db)
            .await
            .unwrap();
    let price = json!({"pattern":"cached-model","label":"cached-model","input_per_1m":300.0,"output_per_1m":8.0,"cached_read_per_1m":0.5});
    let _ = crate::update_model_catalog(
        State(state.clone()),
        axum::extract::Path(entry),
        headers(&db, own, true).await,
        Json(
            serde_json::from_value(
                json!({"pattern":"cached-model","label":"cached-model","price":price}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let response = handle_inner(state.clone(), request(&raw, "cached-model", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!((latest_cost().await - 240.9).abs() < 1e-9);
    // Fresh state without a cache still reads the database directly.
    pricing::set(&state, Vec::new());
    *state.prices.write().unwrap() = None;
    priced(&db, "cached-model", 2.0).await;
    let response = handle_inner(state.clone(), request(&raw, "cached-model", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!((latest_cost().await - 2.5).abs() < 1e-9);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn usage_recorded_while_a_request_queues_stops_it_however_short_the_wait(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "/ok").await;
    let (key_id, raw) = key(&db, own, "all", json!([])).await;
    sqlx::query(
        "UPDATE virtual_api_keys SET concurrency_limit=1,token_limit_daily=100 WHERE id=$1",
    )
    .bind(key_id)
    .execute(&db)
    .await
    .unwrap();
    // Another request holds the only permit; both passed the limit check.
    let permit = state
        .semaphore(key_id, 1)
        .await
        .acquire_owned()
        .await
        .unwrap();
    let queued = tokio::spawn({
        let state = state.clone();
        let raw = raw.clone();
        async move { handle_inner(state, request(&raw, "fixture", false)).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!queued.is_finished());
    // It finishes and records its usage well inside a second.
    sqlx::query("INSERT INTO usage_events(account_id,virtual_key_id,model,protocol,status,duration_ms,input_tokens,output_tokens) VALUES ($1,$2,'fixture','openai',200,10,90,20)")
        .bind(own).bind(key_id).execute(&db).await.unwrap();
    drop(permit);
    match queued.await.unwrap() {
        Err(AppError::Coded(status, "daily_token_limit", _)) => {
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS)
        }
        other => panic!(
            "queued request was not stopped: {:?}",
            other.map(|r| r.status())
        ),
    }
    // A key under its limit still gets through without queueing.
    sqlx::query("UPDATE virtual_api_keys SET token_limit_daily=1000000000 WHERE id=$1")
        .bind(key_id)
        .execute(&db)
        .await
        .unwrap();
    let response = handle_inner(state.clone(), request(&raw, "fixture", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn key_limits_are_answered_from_their_covering_index(db: PgPool) {
    let own = account(&db).await;
    let (id, _) = key(&db, own, "all", json!([])).await;
    sqlx::query("SET enable_seqscan=off")
        .execute(&db)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query_scalar(&format!("EXPLAIN {KEY_USAGE_SQL}"))
        .bind(own)
        .bind(id)
        .bind(None::<DateTime<Utc>>)
        .bind(None::<DateTime<Utc>>)
        .bind(true)
        .fetch_all(&db)
        .await
        .unwrap();
    assert!(
        plan.iter()
            .any(|line| line.contains("usage_key_limits_idx")),
        "{plan:#?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn concurrency_limit_changes_apply_without_a_restart_and_revoked_keys_are_forgotten(
    db: PgPool,
) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let set_limit = |limit: i32| {
        sqlx::query("UPDATE virtual_api_keys SET concurrency_limit=$2 WHERE id=$1")
            .bind(id)
            .bind(limit)
            .execute(&db)
    };
    set_limit(1).await.unwrap();
    let held = state.semaphore(id, 1).await.acquire_owned().await.unwrap();
    let mut queued = tokio::spawn(handle_inner(state.clone(), request(&raw, "fixture", false)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut queued)
            .await
            .is_err()
    );
    // Raising the limit lets the next request through while the old permit is still held.
    set_limit(2).await.unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        handle_inner(state.clone(), request(&raw, "fixture", false)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(state.semaphores.lock().await[&id].1, 2);
    drop(held);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), queued)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .status(),
        200
    );
    crate::revoke_key(
        State(state.clone()),
        axum::extract::Path(id),
        headers(&db, own, false).await,
    )
    .await
    .unwrap();
    assert!(!state.semaphores.lock().await.contains_key(&id));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn retired_test_model_leaves_a_note_until_checks_stop_needing_the_fallback(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let id = channel(&state, own, 10, "allowlist", json!(["fixture-model"]), "").await;
    sqlx::query("INSERT INTO usage_events(account_id,channel_id,model,upstream_model,protocol,status,duration_ms) VALUES ($1,$2,'fixture-model','fixture-model','openai',200,10)")
        .bind(own).bind(id).execute(&db).await.unwrap();
    let note = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT check_note FROM channel_health WHERE channel_id=$1",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap()
    };
    sqlx::query("UPDATE channels SET test_model='retired-model' WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let (healthy, message) = probe_channel(&state, own, id).await.unwrap();
    assert!(healthy, "{message}");
    assert_eq!(message, "检查通过");
    assert_eq!(
        note().await.as_deref(),
        Some("测试模型 retired-model 不可用，已改用 fixture-model 检查。建议更新测试模型。")
    );
    // Traffic does not touch the note; only checks do.
    update_health(&state, id, Health::Success, Source::Traffic)
        .await
        .unwrap();
    assert!(note().await.is_some());
    sqlx::query("UPDATE channels SET test_model='fixture-model' WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(probe_channel(&state, own, id).await.unwrap().0);
    assert!(note().await.is_none());
    server.abort();
}
