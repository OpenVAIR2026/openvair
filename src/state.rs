use crate::{
    crypto::Crypto,
    error::{AppError, Result},
};
use redis::{Client as RedisClient, aio::ConnectionManager};
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{Mutex, OnceCell, Semaphore};
use uuid::Uuid;

/// Each key's concurrency limiter with the limit it was built for.
pub type Semaphores = HashMap<Uuid, (Arc<Semaphore>, usize)>;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: RedisClient,
    /// Shared connection for gateway hot paths, opened on first use.
    pub redis_connection: Arc<OnceCell<ConnectionManager>>,
    pub http: reqwest::Client,
    pub crypto: Arc<Crypto>,
    pub semaphores: Arc<Mutex<Semaphores>>,
    pub cookie_secure: bool,
    pub super_admin_emails: Arc<HashSet<String>>,
    pub routing_settings: Arc<RwLock<crate::routing::RoutingSettings>>,
    /// Model prices for the gateway, filled at startup and kept current by
    /// `pricing::start_refresh`. Unset means read them from the database.
    pub prices: Arc<RwLock<Option<Arc<Vec<crate::pricing::ModelPrice>>>>>,
}

impl AppState {
    /// The key's concurrency limiter. A changed limit replaces the limiter:
    /// a higher limit applies at once; after a lower one, permits already
    /// held on the old limiter last until their requests finish, so the key
    /// may briefly run above the new limit.
    pub async fn semaphore(&self, key_id: Uuid, limit: usize) -> Arc<Semaphore> {
        let limit = limit.max(1);
        let mut all = self.semaphores.lock().await;
        let (semaphore, built_for) = all
            .entry(key_id)
            .or_insert_with(|| (Arc::new(Semaphore::new(limit)), limit));
        if *built_for != limit {
            *semaphore = Arc::new(Semaphore::new(limit));
            *built_for = limit;
        }
        Arc::clone(semaphore)
    }

    /// Drops the limiter of a key that will not be used again.
    pub async fn forget_semaphore(&self, key_id: Uuid) {
        self.semaphores.lock().await.remove(&key_id);
    }

    /// The shared Redis connection. It reconnects by itself after a failure;
    /// a failed first connect is retried on the next call.
    pub async fn redis_connection(&self) -> redis::RedisResult<ConnectionManager> {
        self.redis_connection
            .get_or_try_init(|| {
                self.redis.get_connection_manager_with_config(
                    redis::aio::ConnectionManagerConfig::new()
                        .set_number_of_retries(1)
                        .set_connection_timeout(Duration::from_secs(1))
                        .set_response_timeout(Duration::from_secs(2)),
                )
            })
            .await
            .cloned()
    }

    /// The cached model prices, or a fresh load when nothing is cached.
    pub async fn prices(&self) -> Result<Arc<Vec<crate::pricing::ModelPrice>>> {
        let cached = self
            .prices
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match cached {
            Some(prices) => Ok(prices),
            None => Ok(Arc::new(crate::pricing::load(&self.db).await?)),
        }
    }

    pub fn routing_settings(&self) -> crate::routing::RoutingSettings {
        *self
            .routing_settings
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub async fn redis_rate_limit(&self, key_id: Uuid, rpm: Option<i32>) -> Result<()> {
        let Some(rpm) = rpm.filter(|v| *v > 0) else {
            return Ok(());
        };
        let mut conn = self
            .redis_connection()
            .await
            .map_err(|e| AppError::Internal(format!("redis connection: {e}")))?;
        let redis_key = format!(
            "meta-router:rpm:{key_id}:{}",
            chrono::Utc::now().timestamp() / 60
        );
        let count: i64 = redis::cmd("INCR")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::Internal(format!("redis increment: {e}")))?;
        if count == 1 {
            let _: () = redis::cmd("EXPIRE")
                .arg(&redis_key)
                .arg(90)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::Internal(format!("redis expiry: {e}")))?;
        }
        if count > rpm as i64 {
            return Err(AppError::RateLimited);
        }
        Ok(())
    }
}
