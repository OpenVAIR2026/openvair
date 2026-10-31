use super::*;
use axum::{Json, extract::State, http::HeaderMap};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

fn price(pattern: &str) -> ModelPrice {
    ModelPrice {
        pattern: pattern.into(),
        label: pattern.into(),
        input_per_1m: 0.0,
        output_per_1m: 0.0,
        cached_read_per_1m: 0.0,
        synced_from: None,
    }
}

async fn account(db: &PgPool) -> Uuid {
    sqlx::query_scalar("INSERT INTO accounts(name) VALUES ('pricing test') RETURNING id")
        .fetch_one(db)
        .await
        .unwrap()
}

async fn usage(db: &PgPool, account: Uuid, model: &str, age_days: i32, status: i32) {
    sqlx::query(
        "INSERT INTO usage_events(account_id,model,upstream_model,protocol,status,duration_ms,created_at)
         VALUES ($1,'public-alias',$2,'openai',$3,10,now() - $4 * interval '1 day')",
    )
    .bind(account)
    .bind(model)
    .bind(status)
    .bind(age_days)
    .execute(db)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-pricing.sh"]
async fn discovery_uses_actual_upstream_models_and_exact_prices(db: PgPool) {
    save(&db, &[price("free"), price("priced")]).await.unwrap();
    let first = account(&db).await;
    let second = account(&db).await;
    for (account, model, age, status) in [
        (first, "missing", 1, 200),
        (second, "missing", 2, 503),
        (first, "free", 0, 200),
        (first, "priced-version", 0, 200),
        (first, "old", 31, 200),
        (first, "future", -1, 200),
        (first, "", 0, 200),
    ] {
        usage(&db, account, model, age, status).await;
    }
    let result = recent_unpriced(&db).await.unwrap();
    assert_eq!(
        result
            .models
            .iter()
            .map(|m| m.model.as_str())
            .collect::<Vec<_>>(),
        ["missing", "priced-version"]
    );
    assert_eq!(result.models[0].requests, 2);
    add(&db, price("missing")).await.unwrap();
    assert_eq!(recent_unpriced(&db).await.unwrap().models.len(), 1);
    // A later price does not rewrite any recorded billing snapshot.
    assert!(lookup(&load(&db).await.unwrap(), "priced-version").is_none());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-pricing.sh"]
async fn additions_preserve_prices_and_reject_duplicates_or_invalid_data(db: PgPool) {
    save(&db, &[price("existing")]).await.unwrap();
    let (first, second) = tokio::join!(add(&db, price("first")), add(&db, price("second")));
    first.unwrap();
    second.unwrap();
    assert_eq!(load(&db).await.unwrap().len(), 3);

    let (one, two) = tokio::join!(add(&db, price("same")), add(&db, price("same")));
    assert_ne!(one.is_ok(), two.is_ok());
    assert!(matches!(
        one.err().or(two.err()),
        Some(AppError::Conflict(_))
    ));
    assert_eq!(load(&db).await.unwrap().len(), 4);

    let mut invalid = price("negative");
    invalid.output_per_1m = -1.0;
    assert!(matches!(
        add(&db, invalid).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        add(&db, price("  ")).await,
        Err(AppError::BadRequest(_))
    ));

    sqlx::query("UPDATE system_settings SET value_ciphertext='broken' WHERE key='model_pricing'")
        .execute(&db)
        .await
        .unwrap();
    // Archived legacy JSON is no longer consulted for live pricing.
    assert!(recent_unpriced(&db).await.is_ok());
    assert!(add(&db, price("another")).await.is_ok());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-pricing.sh"]
async fn price_sync_updates_only_the_requested_exact_catalog_model(db: PgPool) {
    save(&db, &[price("sync-model"), price("sync-model-mini")])
        .await
        .unwrap();
    sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES ('sync-unpriced','Unpriced')")
        .execute(&db)
        .await
        .unwrap();
    let fetched: Vec<ModelPrice> = ["sync-model", "sync-unpriced", "sync-unregistered"]
        .map(|name| ModelPrice {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
            cached_read_per_1m: 0.5,
            synced_from: Some("openrouter".into()),
            ..price(name)
        })
        .into();

    assert_eq!(sync_one(&db, &fetched, "sync-model").await.unwrap(), 1);
    let prices = load(&db).await.unwrap();
    assert_eq!(lookup(&prices, "sync-model").unwrap().input_per_1m, 2.0);
    assert_eq!(
        lookup(&prices, "sync-model-mini").unwrap().input_per_1m,
        0.0
    );
    assert!(lookup(&prices, "sync-unpriced").is_none());
    assert_eq!(sync_one(&db, &fetched, "sync-unpriced").await.unwrap(), 1);
    assert_eq!(
        lookup(&load(&db).await.unwrap(), "sync-unpriced")
            .unwrap()
            .synced_from
            .as_deref(),
        Some("openrouter")
    );
    for missing in ["sync-model-mini", "sync-unregistered", "sync-*"] {
        assert_eq!(sync_one(&db, &fetched, missing).await.unwrap(), 0);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM model_catalog WHERE pattern='sync-unregistered'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-pricing.sh"]
async fn preview_is_read_only_and_selected_sync_handles_exact_unpriced_and_free_models(db: PgPool) {
    save(
        &db,
        &[price("changed"), price("same"), price("sibling-mini")],
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO model_catalog(pattern,label) VALUES ('unpriced','unpriced'),('free-new','free-new'),('preview-*','template')")
        .execute(&db).await.unwrap();
    let fetched = vec![
        ModelPrice {
            input_per_1m: 2.0,
            ..price("changed")
        },
        ModelPrice {
            input_per_1m: 3.0,
            ..price("unpriced")
        },
        price("free-new"),
        price("same"),
        price("sibling"),
        price("not-in-catalog"),
    ];
    let preview = preview_sync(&db, &fetched).await.unwrap();
    assert_eq!(
        preview
            .iter()
            .map(|change| change.pattern.as_str())
            .collect::<Vec<_>>(),
        ["changed", "free-new", "unpriced"]
    );
    assert_eq!(
        lookup(&load(&db).await.unwrap(), "changed")
            .unwrap()
            .input_per_1m,
        0.0
    );
    assert!(
        preview
            .iter()
            .find(|change| change.pattern == "free-new")
            .unwrap()
            .current
            .is_none()
    );
    assert_eq!(apply_sync(&db, &preview[..1], &fetched).await.unwrap(), 1);
    let remaining = preview_sync(&db, &fetched).await.unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(lookup(&load(&db).await.unwrap(), "unpriced").is_none());
    assert_eq!(apply_sync(&db, &remaining, &fetched).await.unwrap(), 2);
    assert!(preview_sync(&db, &fetched).await.unwrap().is_empty());
    let prices = load(&db).await.unwrap();
    assert_eq!(lookup(&prices, "changed").unwrap().input_per_1m, 2.0);
    assert_eq!(lookup(&prices, "free-new").unwrap().input_per_1m, 0.0);
    assert_eq!(lookup(&prices, "sibling-mini").unwrap().input_per_1m, 0.0);
    assert!(lookup(&prices, "not-in-catalog").is_none());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL; use scripts/test-model-pricing.sh"]
async fn stale_price_previews_never_partially_apply_or_overwrite_new_edits(db: PgPool) {
    save(&db, &[price("first"), price("second")]).await.unwrap();
    let fetched = vec![
        ModelPrice {
            input_per_1m: 2.0,
            ..price("first")
        },
        ModelPrice {
            input_per_1m: 3.0,
            ..price("second")
        },
    ];
    let preview = preview_sync(&db, &fetched).await.unwrap();
    sqlx::query("UPDATE model_catalog SET input_per_1m=9,updated_at=now() WHERE pattern='second'")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        apply_sync(&db, &preview, &fetched).await,
        Err(AppError::Conflict(_))
    ));
    let prices = load(&db).await.unwrap();
    assert_eq!(lookup(&prices, "first").unwrap().input_per_1m, 0.0);
    assert_eq!(lookup(&prices, "second").unwrap().input_per_1m, 9.0);
    let current = preview_sync(&db, &fetched).await.unwrap();
    let mut changed_reference = fetched.clone();
    changed_reference[0].input_per_1m = 8.0;
    assert!(matches!(
        apply_sync(&db, &current, &changed_reference).await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        apply_sync(&db, &current, &[]).await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        apply_sync(&db, &[], &fetched).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        apply_sync(&db, &[current[0].clone(), current[0].clone()], &fetched).await,
        Err(AppError::BadRequest(_))
    ));
    let mut invalid = current[0].clone();
    invalid.pattern = "first*".into();
    assert!(validate_sync_changes(&[invalid]).is_err());
    sqlx::query("DELETE FROM model_catalog WHERE pattern='second'")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        apply_sync(&db, &current, &fetched).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        lookup(&load(&db).await.unwrap(), "first")
            .unwrap()
            .input_per_1m,
        0.0
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-pricing.sh"]
async fn platform_discovery_and_additions_require_super_admin(db: PgPool) {
    let state = crate::state::AppState {
        db: db.clone(),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
        http: reqwest::Client::new(),
        crypto: Arc::new(crate::crypto::Crypto::from_env().unwrap()),
        semaphores: Arc::new(Mutex::new(HashMap::new())),
        cookie_secure: false,
        super_admin_emails: Arc::default(),
        redis_connection: Default::default(),
        routing_settings: Default::default(),
        prices: Default::default(),
    };
    assert!(matches!(
        crate::get_unpriced_models(State(state.clone()), HeaderMap::new()).await,
        Err(AppError::Unauthorized)
    ));
    assert!(matches!(
        crate::preview_model_pricing_sync(State(state.clone()), HeaderMap::new()).await,
        Err(AppError::Unauthorized)
    ));
    let account_id = account(&db).await;
    let user: Uuid = sqlx::query_scalar("INSERT INTO users(email,password_hash) VALUES ('pricing@example.invalid','test-only') RETURNING id")
        .fetch_one(&db).await.unwrap();
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
        .bind(account_id)
        .bind(user)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
        .bind(user).bind(crate::auth::hash_token("pricing-test-session"))
        .execute(&db).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        "gateway_session=pricing-test-session".parse().unwrap(),
    );
    assert!(matches!(
        crate::get_unpriced_models(State(state.clone()), headers.clone()).await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        crate::add_model_pricing(State(state.clone()), headers.clone(), Json(price("secret")))
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        crate::sync_one_model_pricing(
            State(state.clone()),
            headers.clone(),
            Json(crate::SyncOneInput {
                pattern: "secret".into()
            })
        )
        .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        crate::preview_model_pricing_sync(State(state.clone()), headers.clone()).await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        crate::sync_model_pricing(
            State(state.clone()),
            headers.clone(),
            Json(crate::PriceSyncInput { changes: vec![] })
        )
        .await,
        Err(AppError::Forbidden)
    ));
    sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
        .bind(user)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        crate::get_unpriced_models(State(state.clone()), headers.clone())
            .await
            .is_ok()
    );
    assert!(matches!(
        crate::sync_model_pricing(
            State(state.clone()),
            headers.clone(),
            Json(crate::PriceSyncInput { changes: vec![] })
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    for pattern in ["", "prefix-*"] {
        assert!(matches!(
            crate::sync_one_model_pricing(
                State(state.clone()),
                headers.clone(),
                Json(crate::SyncOneInput {
                    pattern: pattern.into()
                })
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }
    assert!(
        crate::add_model_pricing(State(state), headers, Json(price("allowed")))
            .await
            .is_ok()
    );
}
