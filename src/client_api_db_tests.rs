use super::*;
use axum::body::to_bytes;
use sqlx::PgPool;
use std::sync::Arc;

const PASSWORD: &str = "client-test-password";

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

/// A user with an account and a signed-in browser session.
async fn user(db: &PgPool) -> (Uuid, Uuid, HeaderMap) {
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('client fixture') RETURNING id")
            .fetch_one(db)
            .await
            .unwrap();
    let user: Uuid =
        sqlx::query_scalar("INSERT INTO users(email,password_hash) VALUES ($1,$2) RETURNING id")
            .bind(format!("{}@example.invalid", Uuid::new_v4()))
            .bind(auth::hash_password(PASSWORD).unwrap())
            .fetch_one(db)
            .await
            .unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
        .bind(account)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
    let raw = format!("session-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
        .bind(user).bind(auth::hash_token(&raw)).execute(db).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("gateway_session={raw}").parse().unwrap());
    (account, user, headers)
}

async fn key(state: &AppState, account: Uuid, name: &str, enabled: bool) -> (Uuid, String) {
    let raw = auth::random_token("vair");
    let id = sqlx::query_scalar(
        "INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models,model_mode,enabled,secret_ciphertext)
         VALUES ($1,$2,'vair_test',$3,'[]','all',$4,$5) RETURNING id",
    )
    .bind(account)
    .bind(name)
    .bind(auth::hash_token(&raw))
    .bind(enabled)
    .bind(state.crypto.encrypt(&raw).unwrap())
    .fetch_one(&state.db)
    .await
    .unwrap();
    (id, raw)
}

async fn body(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn start(state: &AppState) -> (String, String) {
    let Json(grant) = authorize(
        State(state.clone()),
        HeaderMap::new(),
        Json(serde_json::from_value(json!({"client_name":"VAIR Desktop · Test"})).unwrap()),
    )
    .await
    .unwrap();
    (
        grant["device_code"].as_str().unwrap().to_owned(),
        grant["user_code"].as_str().unwrap().to_owned(),
    )
}

async fn poll(state: &AppState, device_code: &str) -> (StatusCode, Value) {
    body(
        token(
            State(state.clone()),
            Json(serde_json::from_value(json!({ "device_code": device_code })).unwrap()),
        )
        .await
        .unwrap(),
    )
    .await
}

fn bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers
}

/// Approves a fresh grant and returns the client token.
async fn sign_in(state: &AppState, browser: &HeaderMap) -> String {
    let (device_code, user_code) = start(state).await;
    approve(State(state.clone()), Path(user_code), browser.clone())
        .await
        .unwrap();
    let (status, grant) = poll(state, &device_code).await;
    assert_eq!(status, StatusCode::OK);
    grant["access_token"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn device_grant_issues_one_token_after_browser_approval(db: PgPool) {
    let state = test_state(db.clone());
    let (_, user_id, browser) = user(&db).await;
    let (device_code, user_code) = start(&state).await;

    let (status, pending) = poll(&state, &device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(pending["error"], "authorization_pending");
    assert_eq!(poll(&state, &device_code).await.1["error"], "slow_down");

    // People type the code in lower case and without the dash.
    let typed = user_code.replace('-', "").to_ascii_lowercase();
    let Json(request) = show(State(state.clone()), Path(typed.clone()), browser.clone())
        .await
        .unwrap();
    assert_eq!(request.client_name, "VAIR Desktop · Test");
    assert_eq!(request.user_code, user_code);
    approve(State(state.clone()), Path(typed.clone()), browser.clone())
        .await
        .unwrap();
    assert!(matches!(
        approve(State(state.clone()), Path(typed), browser.clone()).await,
        Err(AppError::NotFound)
    ));

    let (status, grant) = poll(&state, &device_code).await;
    assert_eq!(status, StatusCode::OK);
    let access = grant["access_token"].as_str().unwrap();
    assert!(access.starts_with("vcl_"));
    let stored: String =
        sqlx::query_scalar("SELECT token_hash FROM client_tokens WHERE user_id=$1")
            .bind(user_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(stored, auth::hash_token(access));
    assert_eq!(poll(&state, &device_code).await.1["error"], "invalid_grant");

    let Json(me) = me(State(state.clone()), bearer(access)).await.unwrap();
    assert_eq!(me["client_name"], "VAIR Desktop · Test");
    assert_eq!(me["account_name"], "client fixture");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn denied_and_expired_grants_never_issue_tokens(db: PgPool) {
    let state = test_state(db.clone());
    let (_, _, browser) = user(&db).await;

    let (device_code, user_code) = start(&state).await;
    deny(State(state.clone()), Path(user_code), browser.clone())
        .await
        .unwrap();
    assert_eq!(poll(&state, &device_code).await.1["error"], "access_denied");

    let (device_code, user_code) = start(&state).await;
    sqlx::query("UPDATE client_device_authorizations SET expires_at=now()-interval '1 second' WHERE user_code=$1")
        .bind(user_code.replace('-', ""))
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        approve(State(state.clone()), Path(user_code), browser.clone()).await,
        Err(AppError::NotFound)
    ));
    assert_eq!(poll(&state, &device_code).await.1["error"], "expired_token");
    assert_eq!(
        poll(&state, "vdc_unknown").await.1["error"],
        "invalid_grant"
    );

    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_tokens")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn client_tokens_and_browser_sessions_do_not_cross(db: PgPool) {
    let state = test_state(db.clone());
    let (_, _, browser) = user(&db).await;
    let access = sign_in(&state, &browser).await;

    // Suspending the account stops the client like it stops the console.
    sqlx::query("UPDATE accounts SET disabled_at=now() WHERE id=(SELECT account_id FROM memberships WHERE user_id=(SELECT user_id FROM client_tokens LIMIT 1))")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        auth::current_client(&state, &bearer(&access)).await,
        Err(AppError::Unauthorized)
    ));
    sqlx::query("UPDATE accounts SET disabled_at=NULL")
        .execute(&db)
        .await
        .unwrap();

    // A client token is not a console session, and a cookie is not a client.
    assert!(matches!(
        auth::current_user(&state, &bearer(&access)).await,
        Err(AppError::Unauthorized)
    ));
    let mut cookie = bearer(&access);
    cookie.insert(
        "cookie",
        format!("gateway_session={access}").parse().unwrap(),
    );
    assert!(auth::current_user(&state, &cookie).await.is_err());
    assert!(matches!(
        keys(State(state.clone()), browser.clone()).await,
        Err(AppError::Unauthorized)
    ));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn clients_see_and_reveal_only_their_accounts_enabled_keys(db: PgPool) {
    let state = test_state(db.clone());
    let (account, user_id, browser) = user(&db).await;
    let (other_account, _, _) = user(&db).await;
    let (usable, raw) = key(&state, account, "laptop", true).await;
    let (paused, _) = key(&state, account, "paused", false).await;
    let (revoked, _) = key(&state, account, "revoked", true).await;
    sqlx::query("UPDATE virtual_api_keys SET revoked_at=now() WHERE id=$1")
        .bind(revoked)
        .execute(&db)
        .await
        .unwrap();
    let (foreign, _) = key(&state, other_account, "foreign", true).await;
    let group: Uuid = sqlx::query_scalar(
        "INSERT INTO key_groups(account_id,name) VALUES ($1,'laptops') RETURNING id",
    )
    .bind(account)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE virtual_api_keys SET group_id=$1 WHERE id=$2")
        .bind(group)
        .bind(usable)
        .execute(&db)
        .await
        .unwrap();
    let access = sign_in(&state, &browser).await;

    let Json(listed) = keys(State(state.clone()), bearer(&access)).await.unwrap();
    let mut ids: Vec<Uuid> = listed.iter().map(|key| key.id).collect();
    ids.sort();
    let mut expected = vec![usable, paused];
    expected.sort();
    assert_eq!(ids, expected);
    let groups: Vec<Value> = listed
        .iter()
        .map(|key| serde_json::to_value(key).unwrap()["group"].clone())
        .collect();
    let laptop = listed.iter().position(|key| key.id == usable).unwrap();
    assert_eq!(groups[laptop]["name"], "laptops");
    assert_eq!(groups[1 - laptop], Value::Null);

    let (status, revealed) = body(
        reveal(State(state.clone()), Path(usable), bearer(&access))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revealed["key"], raw);
    let via: Value = sqlx::query_scalar(
        "SELECT metadata FROM audit_logs WHERE action='virtual_key.revealed' AND resource_id=$1 AND user_id=$2",
    )
    .bind(usable)
    .bind(user_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(via["via"], "client");
    assert!(!via.to_string().contains(&raw));

    assert!(matches!(
        reveal(State(state.clone()), Path(paused), bearer(&access)).await,
        Err(AppError::Conflict(_))
    ));
    for hidden in [revoked, foreign] {
        assert!(matches!(
            reveal(State(state.clone()), Path(hidden), bearer(&access)).await,
            Err(AppError::NotFound)
        ));
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn keys_list_models_reached_through_vair_key_channels(db: PgPool) {
    let state = test_state(db.clone());
    let (source_account, _, _) = user(&db).await;
    let (account, _, browser) = user(&db).await;
    sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES ('gpt-a','gpt-a') ON CONFLICT (pattern) DO NOTHING")
        .execute(&db).await.unwrap();
    let upstream: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES ($1,'upstream','openai','http://1.1.1.1/v1','[\"gpt-a\"]') RETURNING id",
    )
    .bind(source_account)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,'test',$2)",
    )
    .bind(upstream)
    .bind(state.crypto.encrypt("sk-test-only").unwrap())
    .execute(&db)
    .await
    .unwrap();
    let (source_key, _) = key(&state, source_account, "issued", true).await;
    sqlx::query(
        "INSERT INTO channels(account_id,name,protocol,base_url,model_mapping,model_mode,kind,source_key_id)
         VALUES ($1,'relay','any','','{}','passthrough','vair_key',$2)",
    )
    .bind(account)
    .bind(source_key)
    .execute(&db)
    .await
    .unwrap();
    let (usable, _) = key(&state, account, "relayed", true).await;
    let (narrow, _) = key(&state, account, "claude only", true).await;
    sqlx::query("UPDATE virtual_api_keys SET allowed_models='[\"claude-*\"]',model_mode='allowlist' WHERE id=$1")
        .bind(narrow).execute(&db).await.unwrap();
    let access = sign_in(&state, &browser).await;

    let Json(listed) = keys(State(state.clone()), bearer(&access)).await.unwrap();
    let models = |id: Uuid| {
        listed
            .iter()
            .find(|key| key.id == id)
            .unwrap()
            .models
            .clone()
    };
    assert_eq!(
        models(usable),
        BTreeMap::from([("openai".to_owned(), BTreeSet::from(["gpt-a".to_owned()]))])
    );
    assert!(models(narrow).is_empty());
    let Json(console) = crate::list_keys(State(state.clone()), browser.clone())
        .await
        .unwrap();
    let known = |id: Uuid| {
        console
            .iter()
            .find(|key| key.id == id)
            .unwrap()
            .known_models
            .clone()
    };
    assert_eq!(known(usable), ["gpt-a"]);
    assert!(known(narrow).is_empty());

    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(source_key)
        .execute(&db)
        .await
        .unwrap();
    let Json(listed) = keys(State(state.clone()), bearer(&access)).await.unwrap();
    assert!(listed.iter().all(|key| key.models.is_empty()));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-client-api.sh"]
async fn sign_out_console_revocation_and_password_change_end_client_access(db: PgPool) {
    let state = test_state(db.clone());
    let (_, user_id, browser) = user(&db).await;

    let access = sign_in(&state, &browser).await;
    sign_out(State(state.clone()), bearer(&access))
        .await
        .unwrap();
    assert!(
        auth::current_client(&state, &bearer(&access))
            .await
            .is_err()
    );

    let access = sign_in(&state, &browser).await;
    let Json(listed) = list_tokens(State(state.clone()), browser.clone())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    // Another user cannot revoke this client.
    let (_, _, stranger) = user(&db).await;
    assert!(matches!(
        revoke_token(State(state.clone()), Path(listed[0].id), stranger).await,
        Err(AppError::NotFound)
    ));
    revoke_token(State(state.clone()), Path(listed[0].id), browser.clone())
        .await
        .unwrap();
    assert!(
        auth::current_client(&state, &bearer(&access))
            .await
            .is_err()
    );

    let access = sign_in(&state, &browser).await;
    crate::password_change::change(
        State(state.clone()),
        browser.clone(),
        Json(
            serde_json::from_value(
                json!({"current_password":PASSWORD,"new_password":"client-test-password-2"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert!(
        auth::current_client(&state, &bearer(&access))
            .await
            .is_err()
    );
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM client_tokens WHERE user_id=$1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(live, 0);
}
