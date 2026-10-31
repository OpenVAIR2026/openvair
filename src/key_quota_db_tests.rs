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

async fn key(db: &PgPool, account: Uuid, limit: Option<f64>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models,cost_limit_total_usd)
         VALUES ($1,'quota key','vair_test',$2,'[]'::jsonb,$3) RETURNING id",
    )
    .bind(account)
    .bind(Uuid::new_v4().to_string())
    .bind(limit)
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

async fn raise(
    state: &AppState,
    headers: &HeaderMap,
    key: Uuid,
    amount: f64,
    note: Option<&str>,
) -> Result<f64> {
    increase(
        State(state.clone()),
        Path(key),
        headers.clone(),
        Json(IncreaseInput {
            amount_usd: amount,
            note: note.map(Into::into),
        }),
    )
    .await
    .map(|Json(body)| body["cost_limit_total_usd"].as_f64().unwrap())
}

async fn owner_events(state: &AppState, headers: &HeaderMap, key: Uuid) -> Result<Vec<QuotaEvent>> {
    owner_history(State(state.clone()), Path(key), headers.clone())
        .await
        .map(|Json(events)| events)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn increases_add_to_the_saved_limit_and_leave_a_record(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "seller").await;
    let headers = session(&state.db, owner, "seller@example.test").await;
    let capped = key(&state.db, owner, Some(10.0)).await;
    let uncapped = key(&state.db, owner, None).await;

    assert_eq!(
        raise(&state, &headers, capped, 5.0, Some(" 收款 ¥36 "))
            .await
            .unwrap(),
        15.0
    );
    assert_eq!(
        raise(&state, &headers, capped, 0.1, None).await.unwrap(),
        15.1
    );
    assert!(matches!(
        raise(&state, &headers, capped, 0.0, None).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        raise(&state, &headers, capped, f64::NAN, None).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        raise(&state, &headers, uncapped, 5.0, None).await,
        Err(AppError::BadRequest(_))
    ));

    let events = owner_events(&state, &headers, capped).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "increase");
    assert_eq!(events[0].amount_usd, Some(0.1));
    assert_eq!(events[0].limit_before_usd, Some(15.0));
    assert_eq!(events[0].limit_after_usd, Some(15.1));
    assert_eq!(events[1].note.as_deref(), Some("收款 ¥36"));
    assert!(
        owner_events(&state, &headers, uncapped)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-key-holder.sh"]
async fn other_accounts_reach_records_only_through_their_own_channels(db: PgPool) {
    let state = test_state(db);
    let seller = account(&state.db, "seller").await;
    let buyer = account(&state.db, "buyer").await;
    let stranger = account(&state.db, "stranger").await;
    let seller_headers = session(&state.db, seller, "seller@example.test").await;
    let buyer_headers = session(&state.db, buyer, "buyer@example.test").await;
    let stranger_headers = session(&state.db, stranger, "stranger@example.test").await;
    let shared = key(&state.db, seller, Some(10.0)).await;
    raise(&state, &seller_headers, shared, 20.0, Some("第二批"))
        .await
        .unwrap();
    let channel: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,kind,source_key_id)
         VALUES ($1,'seller key','any','https://vair.invalid','vair_key',$2) RETURNING id",
    )
    .bind(buyer)
    .bind(shared)
    .fetch_one(&state.db)
    .await
    .unwrap();

    assert!(matches!(
        owner_events(&state, &buyer_headers, shared).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        raise(&state, &buyer_headers, shared, 100.0, None).await,
        Err(AppError::NotFound)
    ));
    let Json(seen) = channel_history(State(state.clone()), Path(channel), buyer_headers.clone())
        .await
        .unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].note.as_deref(), Some("第二批"));
    assert!(matches!(
        channel_history(State(state.clone()), Path(channel), stranger_headers).await,
        Err(AppError::NotFound)
    ));
}
