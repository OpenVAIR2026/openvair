use super::*;
use crate::crypto::Crypto;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

fn test_state(db: PgPool) -> AppState {
    AppState {
        db,
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
        http: reqwest::Client::new(),
        crypto: Arc::new(Crypto::from_env().unwrap()),
        semaphores: Arc::new(Mutex::new(HashMap::new())),
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

async fn key(db: &PgPool, account: Uuid, raw: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models)
         VALUES ($1,'shared key','vair_test',$2,'[]'::jsonb) RETURNING id",
    )
    .bind(account)
    .bind(auth::hash_token(raw))
    .fetch_one(db)
    .await
    .unwrap()
}

/// An owner session in `account` for `email`.
async fn session(db: &PgPool, account: Uuid, email: &str) -> HeaderMap {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(email,password_hash) VALUES ($1,'test-only') RETURNING id",
    )
    .bind(email)
    .fetch_one(db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id,role) VALUES ($1,$2,'owner')")
        .bind(account)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
    let token = format!("session-{user}");
    sqlx::query(
        "INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')",
    )
    .bind(user)
    .bind(auth::hash_token(&token))
    .execute(db)
    .await
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        format!("gateway_session={token}").parse().unwrap(),
    );
    headers
}

async fn share(state: &AppState, headers: &HeaderMap, key: Uuid) -> Result<String> {
    create(State(state.clone()), Path(key), headers.clone())
        .await
        .map(|Json(link)| link.code)
}

async fn preview(state: &AppState, headers: &HeaderMap, code: &str) -> Result<LinkView> {
    resolve(
        State(state.clone()),
        headers.clone(),
        Json(ResolveInput { code: code.into() }),
    )
    .await
    .map(|Json(view)| view)
}

async fn take(state: &AppState, headers: &HeaderMap, code: &str) -> Result<Uuid> {
    accept(
        State(state.clone()),
        headers.clone(),
        Json(AcceptInput {
            code: code.into(),
            name: "Shared".into(),
            priority: Some(5),
            model_mapping: None,
        }),
    )
    .await
    .map(|Json(body)| serde_json::from_value(body["id"].clone()).unwrap())
}

fn is_invalid<T>(result: &Result<T>) -> bool {
    matches!(result, Err(AppError::BadRequest(message)) if message == INVALID_LINK)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn a_link_adds_the_key_as_a_channel_once(db: PgPool) {
    let state = test_state(db.clone());
    let owner = account(&db, "Owner Lab").await;
    let owner_headers = session(&db, owner, "owner@example.invalid").await;
    let shared = key(&db, owner, "vair_shared_test_only").await;
    let caller = account(&db, "Caller Lab").await;
    let caller_headers = session(&db, caller, "caller@example.invalid").await;
    let other = account(&db, "Other Lab").await;
    let other_headers = session(&db, other, "other@example.invalid").await;

    let code = share(&state, &owner_headers, shared).await.unwrap();
    assert!(code.starts_with("vair_share_"));
    // Only the hash is stored.
    let stored: String = sqlx::query_scalar("SELECT code_hash FROM key_share_links")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(stored, auth::hash_token(&code));
    let views = crate::key_views(&state, owner).await.unwrap();
    assert!(views[0].share_expires_at.is_some());

    // The sharer cannot add their own key.
    assert!(matches!(
        preview(&state, &owner_headers, &code).await,
        Err(AppError::BadRequest(message)) if message != INVALID_LINK
    ));
    let view = preview(&state, &caller_headers, &code).await.unwrap();
    assert_eq!(view.source.id, shared);
    let body = serde_json::to_value(&view).unwrap();
    assert_eq!(body["account_name"], "Owner Lab");
    assert!(body["expires_at"].is_string());

    let channel = take(&state, &caller_headers, &code).await.unwrap();
    let (kind, source, priority, availability): (String, Uuid, i32, bool) = sqlx::query_as(
        "SELECT c.kind,c.source_key_id,c.priority,m.availability_enabled
         FROM channels c JOIN channel_monitor_configs m ON m.channel_id=c.id
         WHERE c.id=$1 AND c.account_id=$2",
    )
    .bind(channel)
    .bind(caller)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        (kind.as_str(), source, priority, availability),
        ("vair_key", shared, 5, false)
    );

    // The code is spent.
    assert!(is_invalid(&take(&state, &other_headers, &code).await));
    assert!(is_invalid(&preview(&state, &other_headers, &code).await));

    // The owner sees who accepted, but not accounts that pasted the key.
    sqlx::query(
        "INSERT INTO channels(account_id,name,protocol,base_url,model_mode,kind,source_key_id)
         VALUES ($1,'pasted','any','','passthrough','vair_key',$2)",
    )
    .bind(other)
    .bind(shared)
    .execute(&db)
    .await
    .unwrap();
    let views = crate::key_views(&state, owner).await.unwrap();
    assert_eq!(views[0].channel_count, 2);
    assert_eq!(
        views[0].shared_accounts,
        json!([{"name": "Caller Lab", "email": "caller@example.invalid"}])
    );
    assert!(views[0].share_expires_at.is_none());
    // Once the recipient deletes the channel, it is no longer listed.
    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(channel)
        .execute(&db)
        .await
        .unwrap();
    let views = crate::key_views(&state, owner).await.unwrap();
    assert_eq!(views[0].shared_accounts, json!([]));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn owners_reissue_and_cancel_links_for_their_own_keys(db: PgPool) {
    let state = test_state(db.clone());
    let owner = account(&db, "Owner").await;
    let owner_headers = session(&db, owner, "owner@example.invalid").await;
    let shared = key(&db, owner, "vair_shared_test_only").await;
    let caller = account(&db, "Caller").await;
    let caller_headers = session(&db, caller, "caller@example.invalid").await;

    // Another account can neither share nor cancel the owner's key.
    assert!(matches!(
        share(&state, &caller_headers, shared).await,
        Err(AppError::NotFound)
    ));
    let first = share(&state, &owner_headers, shared).await.unwrap();
    assert!(matches!(
        cancel(State(state.clone()), Path(shared), caller_headers.clone()).await,
        Err(AppError::NotFound)
    ));

    // A new link replaces the open one.
    let second = share(&state, &owner_headers, shared).await.unwrap();
    assert!(is_invalid(&preview(&state, &caller_headers, &first).await));
    assert!(preview(&state, &caller_headers, &second).await.is_ok());

    cancel(State(state.clone()), Path(shared), owner_headers.clone())
        .await
        .unwrap();
    assert!(is_invalid(&take(&state, &caller_headers, &second).await));
    assert!(matches!(
        cancel(State(state.clone()), Path(shared), owner_headers.clone()).await,
        Err(AppError::NotFound)
    ));
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE account_id=$1 ORDER BY created_at,action",
    )
    .bind(owner)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "key_share.created",
            "key_share.created",
            "key_share.cancelled"
        ]
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn links_stop_working_when_they_expire_or_the_key_does(db: PgPool) {
    let state = test_state(db.clone());
    let owner = account(&db, "Owner").await;
    let owner_headers = session(&db, owner, "owner@example.invalid").await;
    let shared = key(&db, owner, "vair_shared_test_only").await;
    let caller = account(&db, "Caller").await;
    let caller_headers = session(&db, caller, "caller@example.invalid").await;

    assert!(is_invalid(
        &preview(&state, &caller_headers, "vair_share_unknown").await
    ));
    assert!(is_invalid(
        &preview(&state, &caller_headers, "vair_shared_test_only").await
    ));

    let code = share(&state, &owner_headers, shared).await.unwrap();
    sqlx::query("UPDATE key_share_links SET expires_at=now()-interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    assert!(is_invalid(&take(&state, &caller_headers, &code).await));

    // A paused key cannot be shared, and pausing it blocks an open link.
    let code = share(&state, &owner_headers, shared).await.unwrap();
    sqlx::query("UPDATE virtual_api_keys SET enabled=false WHERE id=$1")
        .bind(shared)
        .execute(&db)
        .await
        .unwrap();
    assert!(is_invalid(&take(&state, &caller_headers, &code).await));
    assert!(matches!(
        share(&state, &owner_headers, shared).await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("UPDATE virtual_api_keys SET enabled=true WHERE id=$1")
        .bind(shared)
        .execute(&db)
        .await
        .unwrap();
    assert!(preview(&state, &caller_headers, &code).await.is_ok());

    // So does disabling the owner's account.
    sqlx::query("UPDATE accounts SET disabled_at=now() WHERE id=$1")
        .bind(owner)
        .execute(&db)
        .await
        .unwrap();
    assert!(is_invalid(&take(&state, &caller_headers, &code).await));
    sqlx::query("UPDATE accounts SET disabled_at=NULL WHERE id=$1")
        .bind(owner)
        .execute(&db)
        .await
        .unwrap();

    // Revoking the key cancels its open link.
    crate::revoke_key(State(state.clone()), Path(shared), owner_headers.clone())
        .await
        .unwrap();
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM key_share_links WHERE accepted_at IS NULL AND cancelled_at IS NULL",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(open, 0);
    assert!(is_invalid(&take(&state, &caller_headers, &code).await));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn a_failed_accept_leaves_the_link_open(db: PgPool) {
    let state = test_state(db.clone());
    let owner = account(&db, "Owner").await;
    let owner_headers = session(&db, owner, "owner@example.invalid").await;
    let shared = key(&db, owner, "vair_shared_test_only").await;
    let caller = account(&db, "Caller").await;
    let caller_headers = session(&db, caller, "caller@example.invalid").await;

    // The caller already pasted this key.
    let pasted = vair_key_channels::create_channel(
        State(state.clone()),
        caller_headers.clone(),
        Json(
            serde_json::from_value(json!({"name": "Pasted", "secret": "vair_shared_test_only"}))
                .unwrap(),
        ),
    )
    .await
    .map(|Json(body)| body["id"].as_str().unwrap().parse::<Uuid>().unwrap())
    .unwrap();
    let code = share(&state, &owner_headers, shared).await.unwrap();
    assert!(matches!(
        take(&state, &caller_headers, &code).await,
        Err(AppError::Conflict(_))
    ));
    let accepted: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT accepted_at FROM key_share_links")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(accepted.is_none());

    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(pasted)
        .execute(&db)
        .await
        .unwrap();
    take(&state, &caller_headers, &code).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-vair-key-channels.sh"]
async fn concurrent_accepts_use_a_link_once(db: PgPool) {
    let state = test_state(db.clone());
    let owner = account(&db, "Owner").await;
    let owner_headers = session(&db, owner, "owner@example.invalid").await;
    let shared = key(&db, owner, "vair_shared_test_only").await;
    let first = account(&db, "First").await;
    let first_headers = session(&db, first, "first@example.invalid").await;
    let second = account(&db, "Second").await;
    let second_headers = session(&db, second, "second@example.invalid").await;

    let code = share(&state, &owner_headers, shared).await.unwrap();
    let (a, b) = tokio::join!(
        take(&state, &first_headers, &code),
        take(&state, &second_headers, &code)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(is_invalid(if a.is_ok() { &b } else { &a }));
    let channels: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE source_key_id=$1")
        .bind(shared)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(channels, 1);
}
