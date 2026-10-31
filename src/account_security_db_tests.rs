use super::*;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};

const PASSWORD: &str = "account-test-password";

fn test_state(db: PgPool) -> AppState {
    AppState {
        db,
        redis: redis::Client::open(
            std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into()),
        )
        .unwrap(),
        http: reqwest::Client::new(),
        crypto: Arc::new(crypto::Crypto::from_env().unwrap()),
        semaphores: Arc::default(),
        cookie_secure: false,
        super_admin_emails: Arc::default(),
        redis_connection: Default::default(),
        routing_settings: Default::default(),
        prices: Default::default(),
    }
}

async fn user(db: &PgPool) -> (Uuid, Uuid, String) {
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('security fixture') RETURNING id")
            .fetch_one(db)
            .await
            .unwrap();
    let email = format!("{}@example.invalid", Uuid::new_v4());
    let user: Uuid =
        sqlx::query_scalar("INSERT INTO users(email,password_hash) VALUES ($1,$2) RETURNING id")
            .bind(&email)
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
    (account, user, email)
}

async fn session(db: &PgPool, user: Uuid) -> (Uuid, HeaderMap) {
    let raw = format!("session-{}", Uuid::new_v4());
    let id = sqlx::query_scalar("INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour') RETURNING id")
        .bind(user).bind(auth::hash_token(&raw)).fetch_one(db).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("gateway_session={raw}").parse().unwrap());
    (id, headers)
}

/// A plain SMTP sink that records each delivery transcript.
async fn smtp_sink(state: &AppState) -> (Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = json!({"host":"127.0.0.1","port":listener.local_addr().unwrap().port(),
        "security":"test","from":"VAIR <noreply@example.invalid>"});
    sqlx::query("INSERT INTO system_settings(key,value_ciphertext) VALUES ('smtp',$1) ON CONFLICT(key) DO UPDATE SET value_ciphertext=$1")
        .bind(state.crypto.encrypt(&config.to_string()).unwrap()).execute(&state.db).await.unwrap();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let captured = captured.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 test\r\n").await.unwrap();
                let (mut transcript, mut data) = (String::new(), false);
                while let Ok(Some(line)) = lines.next_line().await {
                    transcript.push_str(&line);
                    transcript.push('\n');
                    let reply: &[u8] = if data {
                        if line != "." {
                            continue;
                        }
                        data = false;
                        b"250 OK\r\n"
                    } else if line.to_ascii_uppercase().starts_with("EHLO") {
                        b"250-test\r\n250 OK\r\n"
                    } else if line.to_ascii_uppercase().starts_with("DATA") {
                        data = true;
                        b"354 go ahead\r\n"
                    } else if line.to_ascii_uppercase().starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        b"250 OK\r\n"
                    };
                    write.write_all(reply).await.unwrap();
                }
                captured.lock().await.push(transcript);
            });
        }
    });
    (messages, server)
}

async fn delivered_to(messages: &Mutex<Vec<String>>, email: &str) -> usize {
    let recipient = format!("RCPT TO:<{email}>");
    for _ in 0..100 {
        let count = messages
            .lock()
            .await
            .iter()
            .filter(|message| message.contains(&recipient))
            .count();
        if count > 0 {
            return count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    0
}

fn reset_request(email: &str) -> Json<password_reset::RequestInput> {
    Json(serde_json::from_value(json!({ "email": email })).unwrap())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn reset_requests_reveal_nothing_and_only_registered_users_get_codes(db: PgPool) {
    let state = test_state(db.clone());
    let (messages, server) = smtp_sink(&state).await;
    let (_, user_id, email) = user(&db).await;
    let unknown = format!("{}@example.invalid", Uuid::new_v4());
    let mut shapes = Vec::new();
    for address in [&email, &unknown] {
        let Json(response) = password_reset::request(
            State(state.clone()),
            HeaderMap::new(),
            reset_request(address),
        )
        .await
        .unwrap();
        let mut keys: Vec<_> = response.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        shapes.push(keys);
        Uuid::parse_str(response["request_id"].as_str().unwrap()).unwrap();
    }
    assert_eq!(shapes[0], shapes[1]);
    assert_eq!(delivered_to(&messages, &email).await, 1);
    let pending: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM password_resets")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(pending, [user_id]);
    assert_eq!(delivered_to(&messages, &unknown).await, 0);
    assert!(matches!(
        password_reset::request(
            State(state.clone()),
            HeaderMap::new(),
            reset_request(&email)
        )
        .await,
        Err(AppError::RateLimited)
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn reset_confirmation_replaces_the_password_and_ends_every_session(db: PgPool) {
    let state = test_state(db.clone());
    let (messages, server) = smtp_sink(&state).await;
    let (_, user_id, email) = user(&db).await;
    for _ in 0..2 {
        session(&db, user_id).await;
    }
    let request_id = Uuid::new_v4();
    sqlx::query("INSERT INTO password_resets(user_id,id,code_hash,expires_at) VALUES ($1,$2,$3,now()+interval '10 minutes')")
        .bind(user_id).bind(request_id).bind(auth::hash_password("123456").unwrap()).execute(&db).await.unwrap();
    let confirm = |id: Uuid, address: &str, code: &str, password: &str| {
        password_reset::confirm(
            State(state.clone()),
            Json(
                serde_json::from_value(
                    json!({"request_id":id,"email":address,"code":code,"new_password":password}),
                )
                .unwrap(),
            ),
        )
    };
    let new_password = "reset-new-password";
    for (id, address, code, password) in [
        (Uuid::new_v4(), email.as_str(), "123456", new_password),
        (
            request_id,
            "missing@example.invalid",
            "123456",
            new_password,
        ),
        (request_id, email.as_str(), "123456", "short"),
        (request_id, email.as_str(), "000000", new_password),
    ] {
        assert!(matches!(
            confirm(id, address, code, password).await,
            Err(AppError::BadRequest(_))
        ));
    }
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM password_resets WHERE user_id=$1")
        .bind(user_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    let _ = confirm(request_id, &email, "123456", new_password)
        .await
        .unwrap();
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(auth::verify_password(new_password, &hash));
    for table in ["sessions", "password_resets"] {
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE user_id=$1"))
                .bind(user_id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert!(matches!(
        confirm(request_id, &email, "123456", "another-new-password").await,
        Err(AppError::BadRequest(_))
    ));
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE user_id=$1 AND action='user.password_reset' AND metadata='{}'::jsonb",
    )
    .bind(user_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    // The owner is told about the change by email.
    assert_eq!(delivered_to(&messages, &email).await, 1);
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn failed_sign_ins_are_limited_per_email_and_cleared_by_success(db: PgPool) {
    let state = test_state(db.clone());
    let (_, user_id, email) = user(&db).await;
    let login_as = |address: &str, password: &str, ip: &str| {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", ip.parse().unwrap());
        headers.insert("user-agent", "Security Test Browser".parse().unwrap());
        login(
            State(state.clone()),
            headers,
            Json(auth::LoginInput {
                email: address.into(),
                password: password.into(),
            }),
        )
    };
    for _ in 0..3 {
        assert!(matches!(
            login_as(&email, "wrong-password-value", "198.51.100.1").await,
            Err(AppError::Unauthorized)
        ));
    }
    login_as(&email, PASSWORD, "198.51.100.1").await.unwrap();
    let agent: Option<String> =
        sqlx::query_scalar("SELECT user_agent FROM sessions WHERE user_id=$1")
            .bind(user_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(agent.as_deref(), Some("Security Test Browser"));
    // Success cleared the earlier failures, so ten more are needed to lock.
    for _ in 0..9 {
        assert!(matches!(
            login_as(&email, "wrong-password-value", "198.51.100.2").await,
            Err(AppError::Unauthorized)
        ));
    }
    login_as(&email, PASSWORD, "198.51.100.2").await.unwrap();
    for _ in 0..10 {
        assert!(
            login_as(&email, "wrong-password-value", "198.51.100.3")
                .await
                .is_err()
        );
    }
    assert!(matches!(
        login_as(&email, PASSWORD, "198.51.100.4").await,
        Err(AppError::TooManyAttempts(_))
    ));
    // Unknown addresses are counted the same way and answer like wrong passwords.
    let unknown = format!("{}@example.invalid", Uuid::new_v4());
    for _ in 0..10 {
        assert!(matches!(
            login_as(&unknown, PASSWORD, "198.51.100.5").await,
            Err(AppError::Unauthorized)
        ));
    }
    assert!(matches!(
        login_as(&unknown, PASSWORD, "198.51.100.5").await,
        Err(AppError::TooManyAttempts(_))
    ));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn users_list_and_sign_out_their_other_sessions(db: PgPool) {
    let state = test_state(db.clone());
    let (_, user_id, _) = user(&db).await;
    let (_, other_user, _) = user(&db).await;
    let (current, headers) = session(&db, user_id).await;
    let (second, _) = session(&db, user_id).await;
    let (third, _) = session(&db, user_id).await;
    let (foreign, _) = session(&db, other_user).await;
    let _ = me(State(state.clone()), headers.clone()).await.unwrap();
    let Json(listed) = sessions::list(State(state.clone()), headers.clone())
        .await
        .unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed[0].current && listed[0].id == current && listed[0].last_seen_at.is_some());
    let body = serde_json::to_string(&listed).unwrap();
    assert!(!body.contains("token"));
    let revoke = |id: Uuid| {
        sessions::revoke(
            State(state.clone()),
            axum::extract::Path(id),
            headers.clone(),
        )
    };
    revoke(second).await.unwrap();
    assert!(matches!(
        revoke(current).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(revoke(foreign).await, Err(AppError::NotFound)));
    let Json(result) = sessions::revoke_others(State(state.clone()), headers.clone())
        .await
        .unwrap();
    assert_eq!(result["revoked"], 1);
    let remaining: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM sessions WHERE user_id IN ($1,$2) ORDER BY id")
            .bind(user_id)
            .bind(other_user)
            .fetch_all(&db)
            .await
            .unwrap();
    let mut expected = vec![current, foreign];
    expected.sort();
    assert_eq!(remaining, expected);
    assert!(!remaining.contains(&third));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn users_change_only_their_own_account(db: PgPool) {
    let state = test_state(db.clone());
    let (account, owner, _) = user(&db).await;
    let (other_account, other_user, _) = user(&db).await;
    let load = |id: Uuid| {
        sqlx::query_as::<_, (String, String)>(
            "SELECT name,api_key_prefix FROM accounts WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&db)
    };
    let untouched = load(other_account).await.unwrap();
    let (_, headers) = session(&db, owner).await;
    let _ = update_account(
        State(state.clone()),
        headers.clone(),
        Json(serde_json::from_value(json!({"account_name":"Renamed"})).unwrap()),
    )
    .await
    .unwrap();
    let _ = update_account_settings(
        State(state.clone()),
        headers,
        Json(serde_json::from_value(json!({"api_key_prefix":"team"})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        load(account).await.unwrap(),
        ("Renamed".into(), "team".into())
    );
    assert_eq!(load(other_account).await.unwrap(), untouched);
    // A user has one account, and an account has one user.
    for (account_id, user_id) in [(other_account, owner), (account, other_user)] {
        assert!(
            sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
                .bind(account_id)
                .bind(user_id)
                .execute(&db)
                .await
                .is_err()
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn single_user_migration_stops_on_non_owner_memberships(db: PgPool) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 39)
    {
        sqlx::raw_sql(&migration.sql).execute(&db).await.unwrap();
    }
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('legacy member') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let member: Uuid = sqlx::query_scalar(
        "INSERT INTO users(email,password_hash) VALUES ('legacy@example.invalid','test-only') RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id,role) VALUES ($1,$2,'member')")
        .bind(account)
        .bind(member)
        .execute(&db)
        .await
        .unwrap();
    let migration = include_str!("../migrations/0039_single_user_accounts.sql");
    assert!(sqlx::raw_sql(migration).execute(&db).await.is_err());
    sqlx::query("UPDATE memberships SET role='owner' WHERE user_id=$1")
        .bind(member)
        .execute(&db)
        .await
        .unwrap();
    sqlx::raw_sql(migration).execute(&db).await.unwrap();
    let invitations: Option<String> = sqlx::query_scalar("SELECT to_regclass('invitations')::text")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(invitations.is_none());
    assert!(
        sqlx::query("UPDATE memberships SET role='admin' WHERE user_id=$1")
            .bind(member)
            .execute(&db)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn super_admins_suspend_accounts_and_suspended_users_cannot_sign_in(db: PgPool) {
    let state = test_state(db.clone());
    let (admin_account, admin_id, _) = user(&db).await;
    sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
        .bind(admin_id)
        .execute(&db)
        .await
        .unwrap();
    let (_, admin) = session(&db, admin_id).await;
    let (account, user_id, email) = user(&db).await;
    let (_, member) = session(&db, user_id).await;
    let set_status = |id: Uuid, headers: HeaderMap, body: Value| {
        system_admin::set_account_status(
            State(state.clone()),
            Path(id),
            headers,
            Json(serde_json::from_value(body).unwrap()),
        )
    };
    let login_as = |password: &str| {
        login(
            State(state.clone()),
            HeaderMap::new(),
            Json(auth::LoginInput {
                email: email.clone(),
                password: password.into(),
            }),
        )
    };
    // Only super-admins manage accounts, and never their own or another super-admin's.
    assert!(matches!(
        set_status(admin_account, member.clone(), json!({"disabled":true})).await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        set_status(admin_account, admin.clone(), json!({"disabled":true})).await,
        Err(AppError::BadRequest(_))
    ));
    let _ = set_status(
        account,
        admin.clone(),
        json!({"disabled":true,"reason":" abuse "}),
    )
    .await
    .unwrap();
    let (reason, sessions): (Option<String>, i64) = sqlx::query_as(
        "SELECT a.disabled_reason,(SELECT count(*) FROM sessions WHERE user_id=$2) FROM accounts a WHERE a.id=$1",
    )
    .bind(account)
    .bind(user_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((reason.as_deref(), sessions), (Some("abuse"), 0));
    assert!(matches!(
        auth::current_user(&state, &member).await,
        Err(AppError::Unauthorized)
    ));
    // A wrong password still looks like any failed sign-in.
    assert!(matches!(
        login_as("wrong-password-value").await,
        Err(AppError::Unauthorized)
    ));
    assert!(matches!(
        login_as(PASSWORD).await,
        Err(AppError::Refused("account_disabled", _))
    ));
    let Json(listed) = system_admin::list_accounts(
        State(state.clone()),
        admin.clone(),
        Query(
            serde_json::from_value(json!({"q":email.to_uppercase(),"status":"disabled"})).unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["accounts"][0]["id"], json!(account));
    let _ = set_status(account, admin.clone(), json!({"disabled":false}))
        .await
        .unwrap();
    login_as(PASSWORD).await.unwrap();
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE account_id=$1 AND resource_id=$2 ORDER BY created_at",
    )
    .bind(admin_account)
    .bind(account)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        ["platform.account_disabled", "platform.account_enabled"]
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn registration_can_be_closed_or_limited_to_email_domains(db: PgPool) {
    let mut state = test_state(db.clone());
    let (_, admin_id, _) = user(&db).await;
    sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
        .bind(admin_id)
        .execute(&db)
        .await
        .unwrap();
    let (_, admin) = session(&db, admin_id).await;
    let update = |body: Value| {
        system_admin::update_registration(
            State(state.clone()),
            admin.clone(),
            Json(serde_json::from_value(body).unwrap()),
        )
    };
    let register_as = |state: &AppState, email: &str| {
        register(
            State(state.clone()),
            HeaderMap::new(),
            Json(RegisterInput {
                email: email.into(),
                password: PASSWORD.into(),
                account_name: "registration test".into(),
                request_id: None,
                code: None,
            }),
        )
    };
    let Json(saved) = update(json!({"enabled":true,"allowed_domains":["Allowed.example"]}))
        .await
        .unwrap();
    assert_eq!(saved["allowed_domains"], json!(["allowed.example"]));
    // Without SMTP there is nothing to verify with, so sign-up stays open.
    assert_eq!(saved["require_email_verification"], true);
    assert_eq!(saved["smtp_configured"], false);
    assert!(matches!(
        register_as(&state, "someone@other.example").await,
        Err(AppError::Refused("registration_domain_not_allowed", _))
    ));
    register_as(&state, "someone@allowed.example")
        .await
        .unwrap();
    let _ = update(json!({"enabled":false})).await.unwrap();
    assert!(matches!(
        register_as(&state, "another@allowed.example").await,
        Err(AppError::Refused("registration_closed", _))
    ));
    let Json(status) = system_admin::registration_status(State(state.clone())).await;
    assert_eq!(status["enabled"], false);
    // Configured super-admins can still bootstrap a closed instance.
    state.super_admin_emails = Arc::new(["owner@closed.example".to_owned()].into());
    register_as(&state, "owner@closed.example").await.unwrap();
}

fn register_input(
    email: &str,
    request_id: Option<Uuid>,
    code: Option<&str>,
) -> Json<RegisterInput> {
    Json(RegisterInput {
        email: email.into(),
        password: PASSWORD.into(),
        account_name: "verified signup".into(),
        request_id,
        code: code.map(Into::into),
    })
}

async fn request_signup_code(state: &AppState, email: &str) -> Result<Value> {
    registration::request_code(
        State(state.clone()),
        HeaderMap::new(),
        Json(serde_json::from_value(json!({ "email": email })).unwrap()),
    )
    .await
    .map(|Json(value)| value)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn signup_codes_reveal_nothing_and_only_new_emails_get_codes(db: PgPool) {
    let state = test_state(db.clone());
    let (messages, server) = smtp_sink(&state).await;
    let (_, _, registered) = user(&db).await;
    let fresh = format!("{}@example.invalid", Uuid::new_v4());
    let mut shapes = Vec::new();
    for address in [&registered, &fresh] {
        let response = request_signup_code(&state, address).await.unwrap();
        let mut keys: Vec<_> = response.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        shapes.push(keys);
    }
    assert_eq!(shapes[0], shapes[1]);
    // Both addresses get mail, but only the new one has a pending code.
    assert_eq!(delivered_to(&messages, &registered).await, 1);
    assert_eq!(delivered_to(&messages, &fresh).await, 1);
    let pending: Vec<String> = sqlx::query_scalar("SELECT email FROM registration_verifications")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(pending, std::slice::from_ref(&fresh));
    assert!(matches!(
        request_signup_code(&state, &fresh).await,
        Err(AppError::RateLimited)
    ));
    server.abort();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and Redis; use scripts/test-account-security.sh"]
async fn signup_requires_the_code_sent_to_that_email(db: PgPool) {
    let state = test_state(db.clone());
    let (_, server) = smtp_sink(&state).await;
    let email = format!("{}@example.invalid", Uuid::new_v4());
    let other = format!("{}@example.invalid", Uuid::new_v4());
    let response = request_signup_code(&state, &email).await.unwrap();
    let request_id = Uuid::parse_str(response["request_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE registration_verifications SET code_hash=$1 WHERE email=$2")
        .bind(auth::hash_password("123456").unwrap())
        .bind(&email)
        .execute(&db)
        .await
        .unwrap();
    let attempt = |address: &str, id: Option<Uuid>, code: Option<&str>| {
        register(
            State(state.clone()),
            HeaderMap::new(),
            register_input(address, id, code),
        )
    };
    for (address, id, code) in [
        (email.as_str(), None, None),
        (email.as_str(), Some(Uuid::new_v4()), Some("123456")),
        // A code for one address cannot register another.
        (other.as_str(), Some(request_id), Some("123456")),
        (email.as_str(), Some(request_id), Some("000000")),
    ] {
        assert!(matches!(
            attempt(address, id, code).await,
            Err(AppError::BadRequest(_))
        ));
    }
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM registration_verifications WHERE email=$1")
            .bind(&email)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(users, 0);
    attempt(&email, Some(request_id), Some("123456"))
        .await
        .unwrap();
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM registration_verifications")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(pending, 0);
    // A used code cannot register again, and turning verification off skips it.
    assert!(matches!(
        attempt(&other, Some(request_id), Some("123456")).await,
        Err(AppError::BadRequest(_))
    ));
    sqlx::query("INSERT INTO system_settings(key,value_ciphertext) VALUES ('registration',$1)")
        .bind(json!({"enabled":true,"require_email_verification":false}).to_string())
        .execute(&db)
        .await
        .unwrap();
    attempt(&other, None, None).await.unwrap();
    server.abort();
}
