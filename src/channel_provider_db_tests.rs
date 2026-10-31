use super::*;
use sqlx::PgPool;
use std::sync::Arc;

fn test_state(db: PgPool) -> AppState {
    AppState {
        db,
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
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

async fn signed_in(db: &PgPool) -> (Uuid, HeaderMap) {
    let account: Uuid =
        sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('provider fixture') RETURNING id")
            .fetch_one(db)
            .await
            .unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(email,password_hash) VALUES ($1,'unused') RETURNING id",
    )
    .bind(format!("{}@example.invalid", Uuid::new_v4()))
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
    sqlx::query(
        "INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')",
    )
    .bind(user)
    .bind(auth::hash_token(&raw))
    .execute(db)
    .await
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("gateway_session={raw}").parse().unwrap());
    (account, headers)
}

fn input(protocol: &str, provider: Option<&str>) -> CreateChannelInput {
    CreateChannelInput {
        name: "provider channel".into(),
        protocol: protocol.into(),
        base_url: "https://api.deepseek.com/anthropic".into(),
        priority: None,
        models: Some(vec!["deepseek-v4-flash".into()]),
        model_mode: None,
        test_model: None,
        secret: "sk-test-not-real".into(),
        check_receipt: None,
        provider: provider.map(Into::into),
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-management.sh"]
async fn channels_record_a_matching_provider(db: PgPool) {
    let state = test_state(db.clone());
    let (_, headers) = signed_in(&db).await;
    let (_, other) = signed_in(&db).await;

    let Json(created) = create_channel(
        State(state.clone()),
        headers.clone(),
        Json(input("anthropic", Some("deepseek"))),
    )
    .await
    .unwrap();
    let _ = create_channel(
        State(state.clone()),
        headers.clone(),
        Json(input("openai", None)),
    )
    .await
    .unwrap();
    for (protocol, provider) in [("gemini", "deepseek"), ("openai", "unknown")] {
        let error = create_channel(
            State(state.clone()),
            headers.clone(),
            Json(input(protocol, Some(provider))),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)));
    }

    let Json(channels) = list_channels(State(state.clone()), headers).await.unwrap();
    let providers: Vec<_> = channels
        .iter()
        .map(|channel| (channel.protocol.as_str(), channel.provider.as_deref()))
        .collect();
    assert_eq!(providers.len(), 2);
    assert!(providers.contains(&("anthropic", Some("deepseek"))));
    assert!(providers.contains(&("openai", None)));
    assert_eq!(
        channels
            .iter()
            .find(|channel| channel.provider.is_some())
            .unwrap()
            .id
            .to_string(),
        created["id"].as_str().unwrap()
    );

    // Another account sees none of these channels.
    let Json(foreign) = list_channels(State(state), other).await.unwrap();
    assert!(foreign.is_empty());
}
