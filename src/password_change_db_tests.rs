use super::*;
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

const OLD: &str = "old-password-for-tests";
const NEW: &str = "new-password-for-tests";

fn state(db: PgPool) -> AppState {
    AppState {
        db,
        redis: redis::Client::open(
            std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into()),
        )
        .unwrap(),
        http: reqwest::Client::new(),
        crypto: Arc::new(crate::crypto::Crypto::from_env().unwrap()),
        semaphores: Arc::new(Mutex::new(HashMap::new())),
        cookie_secure: false,
        super_admin_emails: Arc::default(),
        redis_connection: Default::default(),
        routing_settings: Default::default(),
        prices: Default::default(),
    }
}

async fn session(db: &PgPool, user: Uuid) -> (String, HeaderMap) {
    let raw = format!("session-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
        .bind(user).bind(auth::hash_token(&raw)).execute(db).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("gateway_session={raw}").parse().unwrap());
    (auth::hash_token(&raw), headers)
}

async fn fixture(db: &PgPool) -> Uuid {
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('password fixture') RETURNING id")
            .fetch_one(db)
            .await
            .unwrap();
    let user: Uuid =
        sqlx::query_scalar("INSERT INTO users(email,password_hash) VALUES ($1,$2) RETURNING id")
            .bind(format!("{}@example.invalid", Uuid::new_v4()))
            .bind(auth::hash_password(OLD).unwrap())
            .fetch_one(db)
            .await
            .unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
        .bind(account)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
    user
}

async fn change_with(
    state: &AppState,
    headers: &HeaderMap,
    current: &str,
    new: &str,
) -> Result<StatusCode> {
    change(
        State(state.clone()),
        headers.clone(),
        Json(
            serde_json::from_value(json!({"current_password":current,"new_password":new})).unwrap(),
        ),
    )
    .await
}

async fn stored_hash(db: &PgPool, user: Uuid) -> String {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(db)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn members_change_their_password_and_other_sessions_end(db: PgPool) {
    let state = state(db.clone());
    let user = fixture(&db).await;
    let (current_session, headers) = session(&db, user).await;
    let (other_session, other_headers) = session(&db, user).await;
    sqlx::query("INSERT INTO email_changes(user_id,id,session_hash,current_email,new_email,code_hash,expires_at) VALUES ($1,$2,$3,'old@example.invalid','new@example.invalid','test-only',now()+interval '10 minutes')")
        .bind(user).bind(Uuid::new_v4()).bind(&other_session).execute(&db).await.unwrap();
    assert!(matches!(
        change_with(&state, &HeaderMap::new(), OLD, NEW).await,
        Err(AppError::Unauthorized)
    ));
    for (current, new, message) in [
        (OLD, "short", "长度"),
        (OLD, OLD, "不能与当前密码相同"),
        ("wrong-password-value", NEW, "当前密码不正确"),
    ] {
        assert!(matches!(
            change_with(&state, &headers, current, new).await,
            Err(AppError::BadRequest(text)) if text.contains(message)
        ));
    }
    assert!(auth::verify_password(OLD, &stored_hash(&db, user).await));
    assert_eq!(
        change_with(&state, &headers, OLD, NEW).await.unwrap(),
        StatusCode::NO_CONTENT
    );
    let hash = stored_hash(&db, user).await;
    assert!(auth::verify_password(NEW, &hash) && !auth::verify_password(OLD, &hash));
    let sessions: Vec<String> =
        sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=$1")
            .bind(user)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(sessions, [current_session]);
    assert!(matches!(
        change_with(&state, &other_headers, NEW, OLD).await,
        Err(AppError::Unauthorized)
    ));
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM email_changes WHERE user_id=$1")
        .bind(user)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(pending, 0);
    let metadata: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT metadata FROM audit_logs WHERE user_id=$1 AND action='user.password_updated'",
    )
    .bind(user)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(metadata, [json!({})]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn password_guesses_are_limited(db: PgPool) {
    let state = state(db.clone());
    let user = fixture(&db).await;
    let (_, headers) = session(&db, user).await;
    for _ in 0..10 {
        assert!(matches!(
            change_with(&state, &headers, "wrong-password-value", NEW).await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert!(matches!(
        change_with(&state, &headers, OLD, NEW).await,
        Err(AppError::RateLimited)
    ));
    assert!(auth::verify_password(OLD, &stored_hash(&db, user).await));
}
