use super::*;

async fn daily_limit(db: &PgPool, id: Uuid, limit: Option<f64>) {
    sqlx::query("UPDATE virtual_api_keys SET cost_limit_daily_usd=$2 WHERE id=$1")
        .bind(id)
        .bind(limit)
        .execute(db)
        .await
        .unwrap();
}

async fn authenticated(state: &AppState, raw: &str) -> GatewayKey {
    authenticate_virtual_key(state, request(raw, "unused", false).headers())
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn key_writes_without_a_mode_never_allow_every_model(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    // Versions before 0033 write rules without model_mode.
    let legacy: Uuid = sqlx::query_scalar("INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models) VALUES ($1,'legacy','test',$2,'[\"legacy-chat\"]') RETURNING id")
        .bind(own).bind(Uuid::new_v4().to_string()).fetch_one(&db).await.unwrap();
    let (open, _) = key(&db, own, "all", json!([])).await;
    sqlx::query("UPDATE virtual_api_keys SET allowed_models='[\"legacy-chat\"]' WHERE id=$1")
        .bind(open)
        .execute(&db)
        .await
        .unwrap();
    for id in [legacy, open] {
        let mode: String =
            sqlx::query_scalar("SELECT model_mode FROM virtual_api_keys WHERE id=$1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(mode, "allowlist");
    }
    let session = headers(&db, own, false).await;
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(open),
        session,
        Json(serde_json::from_value::<crate::UpdateVirtualKeyInput>(json!({"name":"open","allowed_models":["legacy-chat"],"model_mode":"all","concurrency_limit":4})).unwrap()),
    )
    .await
    .unwrap();
    let saved: (String, Value) =
        sqlx::query_as("SELECT model_mode,allowed_models FROM virtual_api_keys WHERE id=$1")
            .bind(open)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(saved, ("all".into(), json!([])));
    let Json(created) = crate::create_key(
        State(state.clone()),
        headers(&db, own, false).await,
        Json(serde_json::from_value::<crate::CreateVirtualKeyInput>(json!({"name":"null policy","allowed_models":[],"model_mode":"all","unknown_cost_policy":null})).unwrap()),
    )
    .await
    .unwrap();
    assert!(created["id"].is_string());
    // Toggling keeps settings saved meanwhile and never revives revoked keys.
    let toggle = |id: Uuid, enabled: bool| {
        let state = state.clone();
        let db = db.clone();
        async move {
            crate::set_key_enabled(
                State(state),
                axum::extract::Path(id),
                headers(&db, own, false).await,
                Json(serde_json::from_value(json!({ "enabled": enabled })).unwrap()),
            )
            .await
        }
    };
    toggle(open, false).await.unwrap();
    let saved: (bool, String) =
        sqlx::query_as("SELECT enabled,name FROM virtual_api_keys WHERE id=$1")
            .bind(open)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(saved, (false, "open".into()));
    sqlx::query("UPDATE virtual_api_keys SET revoked_at=now() WHERE id=$1")
        .bind(open)
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(toggle(open, true).await, Err(AppError::NotFound)));
    toggle(legacy, true).await.unwrap();
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn streams_always_request_usage_for_cost_accounting(db: PgPool) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "allowlist", json!(["usage-chat"]), "").await;
    priced(&db, "usage-chat", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {raw}"))
        .body(Body::from(
            json!({"model":"usage-chat","messages":[],"stream":true,"stream_options":{"include_usage":false}}).to_string(),
        ))
        .unwrap();
    let response = handle_inner(state.clone(), request).await.unwrap();
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(calls.lock().await[0]["include_usage"], true);
    let mut status = String::new();
    for _ in 0..50 {
        status =
            sqlx::query_scalar("SELECT pricing_status FROM usage_events WHERE virtual_key_id=$1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        if status == "priced" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(status, "priced");
    // The Responses API reports usage on its own and rejects `include_usage`.
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", format!("Bearer {raw}"))
        .body(Body::from(
            json!({"model":"usage-chat","input":"hi","stream":true}).to_string(),
        ))
        .unwrap();
    let response = handle_inner(state.clone(), request).await.unwrap();
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(calls.lock().await[1]["include_usage"], Value::Null);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn key_api_ignores_the_retired_policy_and_rejects_foreign_or_revoked_updates(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let input = json!({"name":"policy key","model_mode":"all","allowed_models":[],"cost_limit_daily_usd":10,"concurrency_limit":4});
    let Json(created) = crate::create_key(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(input.clone()).unwrap()),
    )
    .await
    .unwrap();
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let patch =
        |value| Json(serde_json::from_value::<crate::UpdateVirtualKeyInput>(value).unwrap());
    // Older clients still send the retired setting; it is accepted and ignored.
    let mut blocked = input.clone();
    blocked["unknown_cost_policy"] = json!("block");
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(blocked.clone()),
    )
    .await
    .unwrap();
    let mut old_client = input.clone();
    old_client["enabled"] = json!(false);
    old_client["cost_limit_daily_usd"] = Value::Null;
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(old_client),
    )
    .await
    .unwrap();
    let Json(rows) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert!(!rows[0].enabled);
    let foreign = account(&db).await;
    let foreign_session = headers(&db, foreign, false).await;
    assert!(matches!(
        crate::update_key(
            State(state.clone()),
            axum::extract::Path(id),
            foreign_session,
            patch(input.clone())
        )
        .await,
        Err(AppError::NotFound)
    ));
    crate::revoke_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(
        crate::update_key(
            State(state.clone()),
            axum::extract::Path(id),
            session.clone(),
            patch(input.clone())
        )
        .await,
        Err(AppError::NotFound)
    ));
    assert!(
        crate::list_keys(State(state.clone()), session.clone())
            .await
            .unwrap()
            .0
            .is_empty()
    );
    let Json(created) = crate::create_key(
        State(state.clone()),
        session,
        Json(serde_json::from_value(blocked).unwrap()),
    )
    .await
    .unwrap();
    let saved: String =
        sqlx::query_scalar("SELECT unknown_cost_policy FROM virtual_api_keys WHERE id=$1")
            .bind(Uuid::parse_str(created["id"].as_str().unwrap()).unwrap())
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(saved, "allow");
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn unknown_costs_keep_followup_requests_open_but_known_spend_is_enforced(db: PgPool) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    daily_limit(&db, id, Some(2.0)).await;
    for _ in 0..2 {
        assert_eq!(
            handle_inner(state.clone(), request(&raw, "unpriced", false))
                .await
                .unwrap()
                .status(),
            200
        );
    }
    let rows: Vec<(Option<f64>, String)> =
        sqlx::query_as("SELECT cost_usd,pricing_status FROM usage_events WHERE virtual_key_id=$1")
            .bind(id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![(None, "unpriced".into()), (None, "unpriced".into())]
    );
    priced(&db, "priced", 2.0).await;
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "priced", false))
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(matches!(
        handle_inner(state.clone(), request(&raw, "unpriced", false)).await,
        Err(AppError::Coded(_, "daily_cost_limit", _))
    ));
    assert_eq!(calls.lock().await.len(), 3);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn unpriced_mapped_models_are_served_with_an_unknown_cost(db: PgPool) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let channel_id = channel(&state, own, 10, "allowlist", json!(["public-*"]), "").await;
    sqlx::query("UPDATE channels SET model_mapping=$2 WHERE id=$1")
        .bind(channel_id)
        .bind(json!({"public-alias":"actual-unpriced"}))
        .execute(&db)
        .await
        .unwrap();
    priced(&db, "public-alias", 2.0).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    daily_limit(&db, id, Some(10.0)).await;
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "public-alias", false))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(calls.lock().await[0]["model"], "actual-unpriced");
    // The exact upstream model decides the price, not the public alias.
    let row: (Option<f64>, String) =
        sqlx::query_as("SELECT cost_usd,pricing_status FROM usage_events WHERE virtual_key_id=$1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(row, (None, "unpriced".into()));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn unknown_costs_never_pause_a_key(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    daily_limit(&db, id, Some(10.0)).await;
    for status in ["legacy", "unpriced", "pending_usage"] {
        sqlx::query("INSERT INTO usage_events(account_id,virtual_key_id,model,protocol,status,duration_ms,pricing_status) VALUES ($1,$2,'fixture','openai',200,1,$3)")
            .bind(own).bind(id).bind(status).execute(&db).await.unwrap();
    }
    // Keys that once paused on unknown cost keep working after the setting is gone.
    sqlx::query("UPDATE virtual_api_keys SET unknown_cost_policy='block' WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn disconnected_stream_is_billed_by_drained_upstream_usage(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    priced(&db, "stream-model", 2.0).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "/late-usage").await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    daily_limit(&db, id, Some(100.0)).await;
    let response = handle_inner(state.clone(), request(&raw, "stream-model", true))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    assert!(body.next().await.unwrap().unwrap().starts_with(b"data: "));
    drop(body);
    let (input, cached, cost, status, completion): (i64, i64, f64, String, String) =
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let row = sqlx::query_as("SELECT input_tokens,cached_tokens,cost_usd,pricing_status,request_diagnostics->>'completion' FROM usage_events WHERE virtual_key_id=$1 AND in_flight_until IS NULL")
                    .bind(id).fetch_optional(&db).await.unwrap();
                if let Some(row) = row {
                    break row;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    assert_eq!(
        (input, cached, status.as_str()),
        (1_000_000, 200_000, "priced")
    );
    assert!(cost > 0.0);
    assert_eq!(completion, "client_disconnected");
    // The drained usage leaves no unknown cost to pause the key.
    assert_eq!(
        handle_inner(state.clone(), request(&raw, "stream-model", false))
            .await
            .unwrap()
            .status(),
        200
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn empty_stream_fails_over_and_bills_nothing_on_the_last_channel(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    priced(&db, "stream-model", 2.0).await;
    let own = account(&db).await;
    let empty = channel(&state, own, 10, "passthrough", json!([]), "/empty").await;
    let good = channel(&state, own, 20, "passthrough", json!([]), "").await;
    let (id, raw) = key(&db, own, "all", json!([])).await;
    let response = handle_inner(state.clone(), request(&raw, "stream-model", true))
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.windows(4).any(|w| w == b"\"Hi\""));
    let (channel_id, timing): (
        Uuid,
        sqlx::types::Json<crate::request_diagnostics::Diagnostics>,
    ) = sqlx::query_as(
        "SELECT channel_id,request_diagnostics FROM usage_events WHERE virtual_key_id=$1",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(channel_id, good);
    assert_eq!(timing.attempts[0].channel_id, empty);
    assert_eq!(timing.attempts[0].outcome, Outcome::StreamError);
    // With no channel left, the empty stream is relayed and costs nothing.
    sqlx::query("UPDATE channels SET enabled=false WHERE id=$1")
        .bind(good)
        .execute(&db)
        .await
        .unwrap();
    let (only, only_raw) = key(&db, own, "all", json!([])).await;
    daily_limit(&db, only, Some(100.0)).await;
    let response = handle_inner(state.clone(), request(&only_raw, "stream-model", true))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let (input, output, cost, status): (i64, i64, f64, String) =
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let row = sqlx::query_as("SELECT input_tokens,output_tokens,cost_usd,pricing_status FROM usage_events WHERE virtual_key_id=$1 AND in_flight_until IS NULL AND cost_usd IS NOT NULL")
                    .bind(only).fetch_optional(&db).await.unwrap();
                if let Some(row) = row {
                    break row;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    assert_eq!(
        (input, output, cost, status.as_str()),
        (0, 0, 0.0, "priced")
    );
    assert_eq!(
        handle_inner(state.clone(), request(&only_raw, "stream-model", false))
            .await
            .unwrap()
            .status(),
        200
    );
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn waiting_request_rechecks_limits_after_queueing(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    channel(&state, own, 10, "passthrough", json!([]), "").await;
    priced(&db, "stream-model", 2.0).await;
    for wait_ms in [1_100, 30] {
        let (id, raw) = key(&db, own, "all", json!([])).await;
        daily_limit(&db, id, Some(1.0)).await;
        sqlx::query("UPDATE virtual_api_keys SET concurrency_limit=1 WHERE id=$1")
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
        let permit = state.semaphore(id, 1).await.acquire_owned().await.unwrap();
        let mut waiting = tokio::spawn(handle_inner(
            state.clone(),
            request(&raw, "stream-model", false),
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(wait_ms), &mut waiting)
                .await
                .is_err()
        );
        spend(&db, own, id, Some(5.0), 0).await;
        drop(permit);
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
            .await
            .unwrap()
            .unwrap();
        // Spend recorded while it queued stops it, however short the wait.
        assert!(matches!(
            result,
            Err(AppError::Coded(_, "daily_cost_limit", _))
        ));
        assert!(matches!(
            handle_inner(state.clone(), request(&raw, "stream-model", false)).await,
            Err(AppError::Coded(_, "daily_cost_limit", _))
        ));
    }
    server.abort();
}

async fn spend(db: &PgPool, account: Uuid, key: Uuid, cost: Option<f64>, days_ago: i32) {
    sqlx::query("INSERT INTO usage_events(account_id,virtual_key_id,model,protocol,status,duration_ms,input_tokens,output_tokens,cost_usd,created_at) VALUES ($1,$2,'fixture','openai',200,1,100,20,$3,now()-make_interval(days=>$4))")
        .bind(account).bind(key).bind(cost).bind(days_ago).execute(db).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn total_cost_limit_counts_lifetime_spend_and_survives_omitted_updates(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let input = json!({"name":"total key","model_mode":"all","allowed_models":[],"cost_limit_total_usd":5,"concurrency_limit":4});
    for invalid in [json!(0), json!(-1)] {
        let mut body = input.clone();
        body["cost_limit_total_usd"] = invalid;
        assert!(matches!(
            crate::create_key(
                State(state.clone()),
                session.clone(),
                Json(serde_json::from_value(body).unwrap())
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    let Json(created) = crate::create_key(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(input.clone()).unwrap()),
    )
    .await
    .unwrap();
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let raw = created["key"].as_str().unwrap().to_owned();
    let foreign = account(&db).await;
    spend(&db, own, id, Some(3.0), 2).await;
    spend(&db, own, id, Some(1.5), 0).await;
    spend(&db, own, id, None, 0).await;
    spend(&db, foreign, id, Some(100.0), 0).await;
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    let Json(rows) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(rows[0].cost_limit_total_usd, Some(5.0));
    assert_eq!(rows[0].used_cost_total_usd, 4.5);
    assert_eq!(rows[0].used_cost_today_usd, 1.5);
    assert_eq!(rows[0].used_tokens_today, 240);
    spend(&db, own, id, Some(0.5), 1).await;
    assert!(matches!(
        enforce_limits(&state, &authenticated(&state, &raw).await).await,
        Err(AppError::Coded(_, "total_cost_limit", _))
    ));
    let patch =
        |value| Json(serde_json::from_value::<crate::UpdateVirtualKeyInput>(value).unwrap());
    let mut omitted = input.clone();
    omitted
        .as_object_mut()
        .unwrap()
        .remove("cost_limit_total_usd");
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(omitted),
    )
    .await
    .unwrap();
    assert_eq!(
        authenticated(&state, &raw).await.cost_limit_total_usd,
        Some(5.0)
    );
    let mut raised = input.clone();
    raised["cost_limit_total_usd"] = json!(10);
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(raised),
    )
    .await
    .unwrap();
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    let mut cleared = input;
    cleared["cost_limit_total_usd"] = Value::Null;
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session,
        patch(cleared),
    )
    .await
    .unwrap();
    assert_eq!(authenticated(&state, &raw).await.cost_limit_total_usd, None);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn usage_filters_by_key_within_the_account(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let (first, _) = key(&db, own, "all", json!([])).await;
    let (second, _) = key(&db, own, "all", json!([])).await;
    let (revoked, _) = key(&db, own, "all", json!([])).await;
    let (stale, _) = key(&db, own, "all", json!([])).await;
    let foreign = account(&db).await;
    let (foreign_key, _) = key(&db, foreign, "all", json!([])).await;
    spend(&db, own, first, Some(1.0), 0).await;
    spend(&db, own, first, Some(2.0), 0).await;
    spend(&db, own, second, Some(4.0), 0).await;
    spend(&db, own, revoked, Some(8.0), 0).await;
    spend(&db, own, stale, Some(8.0), 3).await;
    spend(&db, foreign, foreign_key, Some(16.0), 0).await;
    sqlx::query("UPDATE virtual_api_keys SET revoked_at=now() WHERE id=ANY($1)")
        .bind(vec![revoked, stale])
        .execute(&db)
        .await
        .unwrap();
    let query = |key_id: Uuid| {
        axum::extract::Query(
            serde_json::from_value(json!({"range":"24h","key_id":key_id})).unwrap(),
        )
    };
    let Json(summary) = crate::usage::summary(State(state.clone()), session.clone(), query(first))
        .await
        .unwrap();
    assert_eq!(summary["totals"]["requests"], 2);
    assert_eq!(summary["channels"][0]["estimated_cost_usd"], 3.0);
    let mut options: Vec<(String, bool)> = summary["key_options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| {
            (
                option["id"].as_str().unwrap().to_owned(),
                option["revoked"].as_bool().unwrap(),
            )
        })
        .collect();
    options.sort();
    let mut expected = vec![
        (first.to_string(), false),
        (second.to_string(), false),
        (revoked.to_string(), true),
    ];
    expected.sort();
    assert_eq!(options, expected);
    let Json(events) = crate::usage::events(State(state.clone()), session.clone(), query(revoked))
        .await
        .unwrap();
    assert_eq!(events["total"], 1);
    assert_eq!(events["items"][0]["key_revoked"], true);
    assert_eq!(events["items"][0]["virtual_key_id"], revoked.to_string());
    let Json(summary) = crate::usage::summary(State(state.clone()), session, query(foreign_key))
        .await
        .unwrap();
    assert_eq!(summary["totals"]["requests"], 0);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn usage_resets_restart_limit_windows_without_deleting_events(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let input = json!({"name":"reset key","model_mode":"all","allowed_models":[],"token_limit_daily":200,"cost_limit_daily_usd":2,"cost_limit_total_usd":5,"unknown_cost_policy":"block","concurrency_limit":4});
    let Json(created) = crate::create_key(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .unwrap();
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let raw = created["key"].as_str().unwrap().to_owned();
    spend(&db, own, id, Some(3.0), 2).await;
    spend(&db, own, id, Some(1.5), 0).await;
    spend(&db, own, id, None, 0).await;
    let reset = |session: HeaderMap, scope: &str| {
        crate::reset_key_usage(
            State(state.clone()),
            axum::extract::Path(id),
            session,
            Json(serde_json::from_value(json!({ "scope": scope })).unwrap()),
        )
    };
    // Tokens are checked before cost; today's priced spend stays under its cap.
    assert!(matches!(
        enforce_limits(&state, &authenticated(&state, &raw).await).await,
        Err(AppError::Coded(_, "daily_token_limit", _))
    ));
    sqlx::query("UPDATE virtual_api_keys SET token_limit_daily=NULL WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    sqlx::query("UPDATE virtual_api_keys SET token_limit_daily=200 WHERE id=$1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let foreign = account(&db).await;
    assert!(matches!(
        reset(headers(&db, foreign, false).await, "today").await,
        Err(AppError::NotFound)
    ));
    assert!(serde_json::from_value::<crate::UsageResetInput>(json!({"scope":"week"})).is_err());

    reset(session.clone(), "today").await.unwrap();
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    let Json(rows) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(rows[0].used_tokens_today, 0);
    assert_eq!(rows[0].used_cost_today_usd, 0.0);
    assert_eq!(rows[0].used_cost_total_usd, 4.5);
    assert!(rows[0].usage_reset_daily_at.is_some() && rows[0].usage_reset_total_at.is_none());
    spend(&db, own, id, Some(1.0), 0).await;
    assert!(matches!(
        enforce_limits(&state, &authenticated(&state, &raw).await).await,
        Err(AppError::Coded(_, "total_cost_limit", _))
    ));

    reset(session.clone(), "all").await.unwrap();
    enforce_limits(&state, &authenticated(&state, &raw).await)
        .await
        .unwrap();
    let Json(rows) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(rows[0].used_cost_today_usd, 0.0);
    assert_eq!(rows[0].used_cost_total_usd, 0.0);
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_events WHERE virtual_key_id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(kept, 4);
    let audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE account_id=$1 AND action='virtual_key.usage_reset' AND resource_id=$2",
    )
    .bind(own)
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(audits, 2);
    crate::revoke_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reset(session, "all").await,
        Err(AppError::NotFound)
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn keys_route_only_to_selected_channels(db: PgPool) {
    let (state, server, calls) = recorded_state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let preferred = channel(&state, own, 10, "passthrough", json!([]), "/preferred").await;
    let chosen = channel(&state, own, 20, "passthrough", json!([]), "/chosen").await;
    let foreign = account(&db).await;
    let foreign_channel = channel(&state, foreign, 1, "passthrough", json!([]), "/foreign").await;
    let input = |mode: &str, ids: Value| json!({"name":"scoped","model_mode":"all","allowed_models":[],"concurrency_limit":4,"channel_mode":mode,"channel_ids":ids});
    for invalid in [
        input("selected", json!([])),
        input("selected", json!([foreign_channel])),
        input("some", json!([chosen])),
    ] {
        assert!(matches!(
            crate::create_key(
                State(state.clone()),
                session.clone(),
                Json(serde_json::from_value(invalid).unwrap())
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    let Json(created) = crate::create_key(
        State(state.clone()),
        session.clone(),
        Json(serde_json::from_value(input("selected", json!([chosen]))).unwrap()),
    )
    .await
    .unwrap();
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let raw = created["key"].as_str().unwrap().to_owned();
    let response = handle_inner(state.clone(), request(&raw, "scoped-model", false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        calls.lock().await[0]["url"]
            .as_str()
            .unwrap()
            .contains("/chosen/")
    );
    let Json(rows) = crate::list_keys(State(state.clone()), session.clone())
        .await
        .unwrap();
    assert_eq!(rows[0].channel_mode, "selected");
    assert_eq!(rows[0].channel_ids, vec![chosen]);

    let patch =
        |value| Json(serde_json::from_value::<crate::UpdateVirtualKeyInput>(value).unwrap());
    let mut omitted = input("selected", json!([]));
    let object = omitted.as_object_mut().unwrap();
    object.remove("channel_mode");
    object.remove("channel_ids");
    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(omitted),
    )
    .await
    .unwrap();
    assert_eq!(
        authenticated(&state, &raw).await.channel_ids,
        Some(vec![chosen])
    );

    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(input("all", json!([chosen]))),
    )
    .await
    .unwrap();
    assert_eq!(authenticated(&state, &raw).await.channel_ids, None);
    handle_inner(state.clone(), request(&raw, "scoped-model", false))
        .await
        .unwrap();
    assert!(
        calls.lock().await[1]["url"]
            .as_str()
            .unwrap()
            .contains("/preferred/")
    );

    crate::update_key(
        State(state.clone()),
        axum::extract::Path(id),
        session.clone(),
        patch(input("selected", json!([preferred]))),
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(preferred)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(authenticated(&state, &raw).await.channel_ids, Some(vec![]));
    assert!(matches!(
        handle_inner(state.clone(), request(&raw, "scoped-model", false)).await,
        Err(AppError::Coded(_, "model_unavailable", _))
    ));
    assert_eq!(calls.lock().await.len(), 2);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
async fn all_time_usage_starts_at_the_first_matching_request(db: PgPool) {
    let (state, server) = state(db.clone()).await;
    let own = account(&db).await;
    let session = headers(&db, own, false).await;
    let (old_key, _) = key(&db, own, "all", json!([])).await;
    let (new_key, _) = key(&db, own, "all", json!([])).await;
    spend(&db, own, old_key, Some(1.0), 100).await;
    spend(&db, own, old_key, Some(2.0), 0).await;
    spend(&db, own, new_key, Some(4.0), 0).await;
    let foreign = account(&db).await;
    let (foreign_key, _) = key(&db, foreign, "all", json!([])).await;
    spend(&db, foreign, foreign_key, Some(8.0), 400).await;
    let query = |extra: Value| {
        let mut input = json!({"range":"all","utc_offset_minutes":480});
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
    assert_eq!(summary["totals"]["requests"], 3);
    // About 100 days of history uses weekly buckets, ignoring the foreign account.
    assert_eq!(summary["bucket_seconds"], 7 * 86_400);
    let trend = summary["trend"].as_array().unwrap();
    assert!(trend.len() <= 16);
    let charted: i64 = trend
        .iter()
        .map(|point| point["requests"].as_i64().unwrap())
        .sum();
    assert_eq!(charted, 3);
    let Json(events) =
        crate::usage::events(State(state.clone()), session.clone(), query(json!({})))
            .await
            .unwrap();
    assert_eq!(events["total"], 3);
    let Json(scoped) = crate::usage::summary(
        State(state.clone()),
        session.clone(),
        query(json!({"key_id": new_key})),
    )
    .await
    .unwrap();
    assert_eq!(scoped["totals"]["requests"], 1);
    assert_eq!(scoped["bucket_seconds"], 86_400);
    assert_eq!(scoped["trend"].as_array().unwrap().len(), 1);
    let Json(month) = crate::usage::summary(
        State(state.clone()),
        session,
        axum::extract::Query(serde_json::from_value(json!({"range":"30d"})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(month["totals"]["requests"], 2);
    server.abort();
}
