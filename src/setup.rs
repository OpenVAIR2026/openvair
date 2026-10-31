//! First run of a self-hosted instance: creating its one account and owner.
//!
//! Until the owner exists, anyone who can reach the instance could claim it,
//! so setup asks for a one-time token printed to the server log. A deployment
//! without a browser can instead set INITIAL_ADMIN_EMAIL and
//! INITIAL_ADMIN_PASSWORD and the owner is created at startup.
use crate::{
    AppState, auth,
    error::{AppError, Result},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

/// Serializes owner creation so two setups cannot both find no users.
const SETUP_LOCK: i64 = 324022;

/// The token setup requires, while the instance still has no owner.
static SETUP_TOKEN: Mutex<Option<String>> = Mutex::new(None);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupInput {
    token: String,
    email: String,
    password: String,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/setup", get(status).post(setup))
}

/// Creates the owner from the environment, or prints a setup token, when the
/// instance has no users yet.
pub async fn prepare(db: &PgPool) -> Result<()> {
    if has_owner(db).await? {
        return Ok(());
    }
    let email = std::env::var("INITIAL_ADMIN_EMAIL").unwrap_or_default();
    let password = std::env::var("INITIAL_ADMIN_PASSWORD").unwrap_or_default();
    if !email.is_empty() && !password.is_empty() {
        create_owner(db, &email, &password).await?;
        println!("OpenVAIR: created the administrator {email}");
        return Ok(());
    }
    let token = auth::random_token("setup");
    println!(
        "OpenVAIR has no administrator yet. Open /setup in a browser and enter this setup token:\n\n    {token}\n"
    );
    *SETUP_TOKEN.lock().expect("setup token lock") = Some(token);
    Ok(())
}

async fn has_owner(db: impl sqlx::PgExecutor<'_>) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(db)
        .await?)
}

/// Lets the console send a fresh instance to setup instead of sign-in.
async fn status(State(state): State<AppState>) -> Result<Json<Value>> {
    Ok(Json(json!({"needed": !has_owner(&state.db).await?})))
}

async fn setup(
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Json(input): Json<SetupInput>,
) -> Result<Response> {
    let expected = SETUP_TOKEN.lock().expect("setup token lock").clone();
    // Compare digests so the comparison time says nothing about the token.
    let valid = expected
        .is_some_and(|token| auth::hash_token(&token) == auth::hash_token(input.token.trim()));
    if !valid {
        return Err(AppError::Refused(
            "invalid_setup_token",
            "初始化令牌无效，请使用服务启动日志中打印的令牌".into(),
        ));
    }
    let (user_id, account_id, email) =
        create_owner(&state.db, &input.email, &input.password).await?;
    *SETUP_TOKEN.lock().expect("setup token lock") = None;
    let session_token = auth::insert_session(&state.db, user_id, &request_headers).await?;
    let mut headers = HeaderMap::new();
    auth::set_cookie(
        &mut headers,
        &session_token,
        auth::cookie_secure(state.cookie_secure, &request_headers),
    );
    Ok((
        StatusCode::CREATED,
        headers,
        Json(json!({"user_id":user_id,"account_id":account_id,"email":email})),
    )
        .into_response())
}

/// Creates the instance's account and its owner, who administers the instance.
async fn create_owner(db: &PgPool, email: &str, password: &str) -> Result<(Uuid, Uuid, String)> {
    let email = auth::normalize_email(email)?;
    auth::validate_password(password)?;
    let password_hash = auth::hash_password(password)?;
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SETUP_LOCK)
        .execute(&mut *tx)
        .await?;
    if has_owner(&mut *tx).await? {
        return Err(AppError::Conflict("实例已经初始化，请直接登录".into()));
    }
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users(email,password_hash,is_super_admin) VALUES ($1,$2,true) RETURNING id",
    )
    .bind(&email)
    .bind(password_hash)
    .fetch_one(&mut *tx)
    .await?;
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts(name,api_key_prefix) VALUES ('OpenVAIR','openvair') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
        .bind(account_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((user_id, account_id, email))
}

/// `openvair admin reset-password <email>`: sets a new password read from
/// standard input and signs the user out everywhere. A self-hosted instance
/// may have no mail server to send reset codes, so the operator resets from
/// the host instead.
pub async fn run_admin_command(db: &PgPool, args: &[String]) -> Result<()> {
    let [command, email] = args else {
        return Err(AppError::BadRequest(
            "usage: openvair admin reset-password <email>".into(),
        ));
    };
    if command != "reset-password" {
        return Err(AppError::BadRequest(format!(
            "unknown admin command: {command}"
        )));
    }
    eprintln!("New password for {email} (10-256 characters), then Enter:");
    let mut password = String::new();
    std::io::stdin()
        .read_line(&mut password)
        .map_err(|e| AppError::Internal(format!("read password: {e}")))?;
    reset_password(db, email, password.trim_end_matches(['\r', '\n'])).await?;
    eprintln!("Password reset; every session of {email} is signed out.");
    Ok(())
}

async fn reset_password(db: &PgPool, email: &str, password: &str) -> Result<()> {
    let email = auth::normalize_email(email)?;
    auth::validate_password(password)?;
    let password_hash = auth::hash_password(password)?;
    let mut tx = db.begin().await?;
    let user_id: Uuid =
        sqlx::query_scalar("UPDATE users SET password_hash=$2 WHERE email=$1 RETURNING id")
            .bind(&email)
            .bind(password_hash)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use std::sync::Arc;

    fn test_state(db: PgPool) -> AppState {
        AppState {
            db,
            redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
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

    fn input(token: &str) -> Json<SetupInput> {
        Json(SetupInput {
            token: token.into(),
            email: "Owner@Example.invalid".into(),
            password: "setup-test-password".into(),
        })
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires isolated PostgreSQL; use scripts/test-model-management.sh"]
    async fn only_the_logged_token_creates_the_one_owner(db: PgPool) {
        let state = test_state(db.clone());
        assert_eq!(
            status(State(state.clone())).await.unwrap().0["needed"],
            true
        );
        prepare(&db).await.unwrap();
        let token = SETUP_TOKEN.lock().unwrap().clone().unwrap();

        let wrong = setup(State(state.clone()), HeaderMap::new(), input("setup_wrong")).await;
        assert!(matches!(
            wrong,
            Err(AppError::Refused("invalid_setup_token", _))
        ));

        let response = setup(State(state.clone()), HeaderMap::new(), input(&token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(response.headers().contains_key("set-cookie"));
        let (email, admin, accounts): (String, bool, i64) = sqlx::query_as(
            "SELECT u.email,u.is_super_admin,(SELECT count(*) FROM accounts)
             FROM users u JOIN memberships m ON m.user_id=u.id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(
            (email.as_str(), admin, accounts),
            ("owner@example.invalid", true, 1)
        );

        // The token is spent, and nothing creates a second owner.
        assert!(SETUP_TOKEN.lock().unwrap().is_none());
        let again = setup(State(state.clone()), HeaderMap::new(), input(&token)).await;
        assert!(matches!(again, Err(AppError::Refused(..))));
        let direct = create_owner(&db, "other@example.invalid", "setup-test-password").await;
        assert!(matches!(direct, Err(AppError::Conflict(_))));
        assert_eq!(status(State(state)).await.unwrap().0["needed"], false);

        let user: Uuid = sqlx::query_scalar("SELECT id FROM users")
            .fetch_one(&db)
            .await
            .unwrap();
        crate::auth::insert_session(&db, user, &HeaderMap::new())
            .await
            .unwrap();
        reset_password(&db, "OWNER@example.invalid", "a-new-setup-password")
            .await
            .unwrap();
        let (hash, sessions): (String, i64) =
            sqlx::query_as("SELECT password_hash,(SELECT count(*) FROM sessions) FROM users")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(auth::verify_password("a-new-setup-password", &hash));
        assert_eq!(sessions, 0);
        let missing = reset_password(&db, "nobody@example.invalid", "a-new-setup-password").await;
        assert!(matches!(missing, Err(AppError::NotFound)));
    }
}
