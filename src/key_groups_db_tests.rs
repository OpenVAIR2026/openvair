use super::*;
use crate::crypto::Crypto;
use axum::extract::{Path, State};
use sqlx::PgPool;
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

async fn channel(db: &PgPool, account: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,models) VALUES ($1,$2,'openai','http://1.1.1.1/v1','[\"gpt-a\"]') RETURNING id",
    )
    .bind(account)
    .bind(name)
    .fetch_one(db)
    .await
    .unwrap()
}

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

async fn new_group(state: &AppState, headers: &HeaderMap, input: Value) -> Result<Uuid> {
    let Json(created) = create(
        State(state.clone()),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await?;
    Ok(serde_json::from_value(created["id"].clone()).unwrap())
}

async fn edit_group(state: &AppState, headers: &HeaderMap, id: Uuid, input: Value) -> Result<()> {
    update(
        State(state.clone()),
        Path(id),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .map(|_| ())
}

async fn new_key(state: &AppState, headers: &HeaderMap, input: Value) -> Result<Uuid> {
    let Json(created) = crate::create_key(
        State(state.clone()),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await?;
    Ok(serde_json::from_value(created["id"].clone()).unwrap())
}

async fn edit_key(state: &AppState, headers: &HeaderMap, id: Uuid, input: Value) -> Result<()> {
    crate::update_key(
        State(state.clone()),
        Path(id),
        headers.clone(),
        Json(serde_json::from_value(input).unwrap()),
    )
    .await
    .map(|_| ())
}

/// The key's own scope, as the gateway reads it.
async fn scope(db: &PgPool, key: Uuid) -> (Option<Uuid>, String, Vec<Uuid>, String, Value) {
    sqlx::query_as(
        "SELECT v.group_id,v.channel_mode,
           ARRAY(SELECT vc.channel_id FROM virtual_key_channels vc WHERE vc.virtual_key_id=v.id ORDER BY vc.channel_id),
           v.model_mode,v.allowed_models
         FROM virtual_api_keys v WHERE v.id=$1",
    )
    .bind(key)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-management.sh"]
async fn group_scope_follows_members_until_they_leave(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "teacher").await;
    let headers = session(&state.db, owner, "teacher@example.test").await;
    let first = channel(&state.db, owner, "first").await;
    let second = channel(&state.db, owner, "second").await;

    let group = new_group(
        &state,
        &headers,
        json!({"name":"学生班","channel_mode":"selected","channel_ids":[first],"model_mode":"allowlist","allowed_models":["gpt-a"]}),
    )
    .await
    .unwrap();
    // Submitted scope is ignored for a grouped key.
    let student = new_key(
        &state,
        &headers,
        json!({"name":"student","allowed_models":["other"],"model_mode":"allowlist","channel_mode":"selected","channel_ids":[second],"group_id":group}),
    )
    .await
    .unwrap();
    let alone = new_key(
        &state,
        &headers,
        json!({"name":"alone","allowed_models":[],"model_mode":"all"}),
    )
    .await
    .unwrap();
    assert_eq!(
        scope(&state.db, student).await,
        (
            Some(group),
            "selected".into(),
            vec![first],
            "allowlist".into(),
            json!(["gpt-a"])
        )
    );

    // A joining key takes the scope; editing the group reaches every member.
    edit_key(
        &state,
        &headers,
        alone,
        json!({"name":"alone","allowed_models":[],"model_mode":"all","concurrency_limit":4,"group_id":group}),
    )
    .await
    .unwrap();
    edit_group(
        &state,
        &headers,
        group,
        json!({"name":"学生班","channel_mode":"selected","channel_ids":[second],"model_mode":"allowlist","allowed_models":["gpt-b","gpt-a"]}),
    )
    .await
    .unwrap();
    for key in [student, alone] {
        assert_eq!(
            scope(&state.db, key).await,
            (
                Some(group),
                "selected".into(),
                vec![second],
                "allowlist".into(),
                json!(["gpt-a", "gpt-b"])
            )
        );
    }

    // Older clients omit the group and cannot change a member's scope.
    edit_key(
        &state,
        &headers,
        student,
        json!({"name":"student","allowed_models":[],"model_mode":"all","channel_mode":"all","concurrency_limit":2}),
    )
    .await
    .unwrap();
    assert_eq!(scope(&state.db, student).await.3, "allowlist");

    // Leaving takes the submitted scope; deleting the group keeps the copied one.
    edit_key(
        &state,
        &headers,
        student,
        json!({"name":"student","allowed_models":[],"model_mode":"all","channel_mode":"all","concurrency_limit":2,"group_id":null}),
    )
    .await
    .unwrap();
    assert_eq!(
        scope(&state.db, student).await,
        (None, "all".into(), vec![], "all".into(), json!([]))
    );
    delete(State(state.clone()), Path(group), headers.clone())
        .await
        .unwrap();
    assert_eq!(
        scope(&state.db, alone).await,
        (
            None,
            "selected".into(),
            vec![second],
            "allowlist".into(),
            json!(["gpt-a", "gpt-b"])
        )
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-management.sh"]
async fn groups_stay_inside_their_account(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "owner").await;
    let other = account(&state.db, "other").await;
    let headers = session(&state.db, owner, "owner@example.test").await;
    let other_headers = session(&state.db, other, "other@example.test").await;
    let all = json!({"name":"Class","channel_mode":"all","model_mode":"all"});
    let group = new_group(&state, &headers, all.clone()).await.unwrap();

    assert!(matches!(
        new_group(
            &state,
            &headers,
            json!({"name":" class ","channel_mode":"all","model_mode":"all"})
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        new_group(
            &state,
            &headers,
            json!({"name":"Empty","channel_mode":"all","model_mode":"allowlist"})
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let foreign = channel(&state.db, other, "foreign").await;
    assert!(matches!(
        new_group(&state, &headers, json!({"name":"Foreign","channel_mode":"selected","channel_ids":[foreign],"model_mode":"all"})).await,
        Err(AppError::BadRequest(_))
    ));

    // The other account sees, edits, deletes and joins none of it.
    let Json(listed) = list(State(state.clone()), other_headers.clone())
        .await
        .unwrap();
    assert!(listed.is_empty());
    assert!(matches!(
        edit_group(&state, &other_headers, group, all.clone()).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        delete(State(state.clone()), Path(group), other_headers.clone()).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        new_key(
            &state,
            &other_headers,
            json!({"name":"k","allowed_models":[],"model_mode":"all","group_id":group})
        )
        .await,
        Err(AppError::BadRequest(_))
    ));

    // Revoked members are not counted.
    let member = new_key(
        &state,
        &headers,
        json!({"name":"k","allowed_models":[],"model_mode":"all","group_id":group}),
    )
    .await
    .unwrap();
    new_key(
        &state,
        &headers,
        json!({"name":"k2","allowed_models":[],"model_mode":"all","group_id":group}),
    )
    .await
    .unwrap();
    crate::revoke_key(State(state.clone()), Path(member), headers.clone())
        .await
        .unwrap();
    let Json(listed) = list(State(state.clone()), headers.clone()).await.unwrap();
    assert_eq!(listed[0].key_count, 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-management.sh"]
async fn group_members_are_picked_on_the_group(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "teacher").await;
    let other = account(&state.db, "other").await;
    let headers = session(&state.db, owner, "teacher@example.test").await;
    let other_headers = session(&state.db, other, "other@example.test").await;
    let first = channel(&state.db, owner, "first").await;
    let plain = json!({"name":"k","allowed_models":[],"model_mode":"all"});
    let leaver = new_key(&state, &headers, plain.clone()).await.unwrap();
    let solo = new_key(&state, &headers, plain.clone()).await.unwrap();
    let mover = new_key(&state, &headers, plain.clone()).await.unwrap();
    let foreign = new_key(&state, &other_headers, plain).await.unwrap();

    let class = new_group(
        &state,
        &headers,
        json!({"name":"Class","channel_mode":"selected","channel_ids":[first],"model_mode":"allowlist","allowed_models":["gpt-a"],"key_ids":[leaver]}),
    )
    .await
    .unwrap();
    let other_group = new_group(
        &state,
        &headers,
        json!({"name":"Other","channel_mode":"all","model_mode":"all","key_ids":[mover]}),
    )
    .await
    .unwrap();
    let class_scope = (
        "selected".to_owned(),
        vec![first],
        "allowlist".to_owned(),
        json!(["gpt-a"]),
    );
    let without_group = |s: (Option<Uuid>, String, Vec<Uuid>, String, Value)| (s.1, s.2, s.3, s.4);
    assert_eq!(scope(&state.db, leaver).await.0, Some(class));
    assert_eq!(without_group(scope(&state.db, leaver).await), class_scope);

    // Picking members moves keys from other groups and drops the unpicked.
    let class_input = json!({"name":"Class","channel_mode":"selected","channel_ids":[first],"model_mode":"allowlist","allowed_models":["gpt-a"]});
    let mut picked = class_input.clone();
    picked["key_ids"] = json!([solo, mover, solo]);
    edit_group(&state, &headers, class, picked).await.unwrap();
    for key in [solo, mover] {
        assert_eq!(scope(&state.db, key).await.0, Some(class));
        assert_eq!(without_group(scope(&state.db, key).await), class_scope);
    }
    assert_eq!(scope(&state.db, leaver).await.0, None);
    assert_eq!(without_group(scope(&state.db, leaver).await), class_scope);

    // Omitting members keeps them; another account's key cannot be picked.
    edit_group(&state, &headers, class, class_input.clone())
        .await
        .unwrap();
    let mut stolen = class_input;
    stolen["key_ids"] = json!([foreign]);
    assert!(matches!(
        edit_group(&state, &headers, class, stolen).await,
        Err(AppError::BadRequest(_))
    ));
    let Json(listed) = list(State(state.clone()), headers.clone()).await.unwrap();
    let count = |id| listed.iter().find(|g| g.id == id).unwrap().key_count;
    assert_eq!((count(class), count(other_group)), (2, 0));
    assert_eq!(scope(&state.db, foreign).await.0, None);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires isolated PostgreSQL and a test MASTER_KEY; use scripts/test-model-management.sh"]
async fn a_key_keeps_its_holder_until_it_is_changed_or_cleared(db: PgPool) {
    let state = test_state(db);
    let owner = account(&state.db, "admin").await;
    let headers = session(&state.db, owner, "admin@example.test").await;
    let holder = |id| {
        let db = state.db.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT holder FROM virtual_api_keys WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let base =
        json!({"name":"laptop","allowed_models":[],"model_mode":"all","concurrency_limit":4});

    let mut input = base.clone();
    input["holder"] = json!("  张三  ");
    let key = new_key(&state, &headers, input).await.unwrap();
    assert_eq!(holder(key).await.as_deref(), Some("张三"));

    // Omitting the holder keeps it.
    edit_key(&state, &headers, key, base.clone()).await.unwrap();
    assert_eq!(holder(key).await.as_deref(), Some("张三"));

    let mut input = base.clone();
    input["holder"] = json!("");
    edit_key(&state, &headers, key, input).await.unwrap();
    assert_eq!(holder(key).await, None);

    let mut input = base;
    input["holder"] = json!("x".repeat(121));
    assert!(new_key(&state, &headers, input).await.is_err());
}
