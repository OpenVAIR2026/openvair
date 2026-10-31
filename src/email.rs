use crate::{
    edition::PRODUCT,
    error::{AppError, Result},
    state::AppState,
};
use lettre::{
    Message, SmtpTransport, Transport, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

pub async fn load_smtp_config_optional(state: &AppState) -> Result<Option<SmtpConfig>> {
    let ciphertext = sqlx::query_scalar::<_, String>(
        "SELECT value_ciphertext FROM system_settings WHERE key='smtp'",
    )
    .fetch_optional(&state.db)
    .await?;
    let Some(ciphertext) = ciphertext else {
        return Ok(None);
    };
    let plaintext = state
        .crypto
        .decrypt(&ciphertext)
        .map_err(|_| AppError::ServiceUnavailable("邮箱验证服务配置无效".into()))?;
    let config = serde_json::from_str::<SmtpConfig>(&plaintext)
        .map_err(|_| AppError::ServiceUnavailable("邮箱验证服务配置无效".into()))?;
    Ok(Some(config))
}

pub async fn load_smtp_config(state: &AppState) -> Result<Arc<SmtpConfig>> {
    load_smtp_config_optional(state)
        .await?
        .map(Arc::new)
        .ok_or_else(|| {
            AppError::ServiceUnavailable("邮箱验证服务尚未配置，请联系超级管理员".into())
        })
}

async fn send_message(
    config: Arc<SmtpConfig>,
    recipient: String,
    subject: &'static str,
    body: String,
) -> Result<()> {
    let from: Mailbox = config
        .from
        .parse()
        .map_err(|_| AppError::ServiceUnavailable("邮箱验证服务配置无效".into()))?;
    let to: Mailbox = recipient
        .parse()
        .map_err(|_| AppError::BadRequest("valid email is required".into()))?;
    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(format!("{PRODUCT} {subject}"))
        .body(body)
        .map_err(|_| AppError::ServiceUnavailable("邮箱验证服务不可用".into()))?;
    let mut transport = match config.security.as_str() {
        "tls" => SmtpTransport::relay(&config.host)
            .map_err(|error| {
                eprintln!("smtp transport initialization failed: {error}");
                AppError::ServiceUnavailable("邮箱验证服务不可用，请检查 SMTP 配置".into())
            })?
            .port(config.port),
        "starttls" => SmtpTransport::starttls_relay(&config.host)
            .map_err(|error| {
                eprintln!("smtp transport initialization failed: {error}");
                AppError::ServiceUnavailable("邮箱验证服务不可用，请检查 SMTP 配置".into())
            })?
            .port(config.port),
        #[cfg(test)]
        "test" if config.host == "127.0.0.1" => {
            SmtpTransport::builder_dangerous(&config.host).port(config.port)
        }
        _ => return Err(AppError::ServiceUnavailable("邮箱验证服务配置无效".into())),
    };
    if let (Some(username), Some(password)) = (config.username.clone(), config.password.clone()) {
        transport = transport.credentials(Credentials::new(username, password));
    }
    let transport = transport.timeout(Some(Duration::from_secs(15))).build();
    let result = tokio::task::spawn_blocking(move || transport.send(&message))
        .await
        .map_err(|_| AppError::ServiceUnavailable("邮箱验证服务不可用".into()))?;
    result.map_err(|error| {
        eprintln!(
            "smtp delivery failed (host={}, port={}, security={}, auth_configured={}): {error}",
            config.host,
            config.port,
            config.security,
            config.username.is_some(),
        );
        AppError::ServiceUnavailable("邮箱验证服务发送失败，请检查 SMTP 配置".into())
    })?;
    Ok(())
}

pub async fn send_verification_code(
    config: Arc<SmtpConfig>,
    recipient: String,
    code: String,
) -> Result<()> {
    send_message(
        config,
        recipient,
        "邮箱验证码",
        format!(
            "你的 {PRODUCT} 邮箱验证码是：{code}\n验证码 10 分钟内有效。如非本人操作，请忽略此邮件。"
        ),
    )
    .await
}

pub async fn send_password_reset_code(
    config: Arc<SmtpConfig>,
    recipient: String,
    code: String,
) -> Result<()> {
    send_message(
        config,
        recipient,
        "重置密码验证码",
        format!("你正在重置 {PRODUCT} 登录密码，验证码是：{code}\n验证码 10 分钟内有效。如非本人操作，请忽略此邮件，你的密码不会改变。"),
    )
    .await
}

pub async fn send_registration_code(
    config: Arc<SmtpConfig>,
    recipient: String,
    code: String,
) -> Result<()> {
    send_message(
        config,
        recipient,
        "注册验证码",
        format!(
            "你正在注册 {PRODUCT}，验证码是：{code}\n验证码 10 分钟内有效。如非本人操作，请忽略此邮件。"
        ),
    )
    .await
}

pub async fn send_public_check_code(
    config: Arc<SmtpConfig>,
    recipient: String,
    code: String,
) -> Result<()> {
    send_message(
        config,
        recipient,
        "模型验真验证码",
        format!(
            "你正在使用 {PRODUCT} 模型验真，验证码是：{code}\n验证码 10 分钟内有效。如非本人操作，请忽略此邮件。"
        ),
    )
    .await
}

/// Sent instead of a code when the address already has an account.
pub async fn send_registration_existing(config: Arc<SmtpConfig>, recipient: String) -> Result<()> {
    send_message(
        config,
        recipient,
        "注册提醒",
        format!("有人使用此邮箱申请注册 {PRODUCT}，但该邮箱已有帐号。\n你可以直接登录；如果忘记密码，请在登录页使用“忘记密码”重置。如非本人操作，请忽略此邮件。"),
    )
    .await
}

/// Security notices are best effort: they never block or fail the change itself.
pub fn notify_in_background(state: &AppState, recipient: String, notice: SecurityNotice) {
    let state = state.clone();
    tokio::spawn(async move {
        let Ok(Some(config)) = load_smtp_config_optional(&state).await else {
            return;
        };
        let (subject, body) = match notice {
            SecurityNotice::EmailChanged { new_email } => (
                "登录邮箱已修改",
                format!(
                    "你的 {PRODUCT} 登录邮箱已修改为 {new_email}，其他设备上的登录已退出。\n如非本人操作，请立即联系管理员。"
                ),
            ),
            SecurityNotice::PasswordChanged => (
                "登录密码已修改",
                format!(
                    "你的 {PRODUCT} 登录密码已修改，其他设备上的登录已退出。\n如非本人操作，请立即通过“忘记密码”重置密码并联系管理员。"
                ),
            ),
        };
        if send_message(Arc::new(config), recipient, subject, body)
            .await
            .is_err()
        {
            eprintln!("security notice delivery failed");
        }
    });
}

/// Monitoring alerts are best effort and need SMTP to be configured.
pub fn alert_in_background(
    state: &AppState,
    recipient: String,
    subject: &'static str,
    body: String,
) {
    let state = state.clone();
    tokio::spawn(async move {
        let Ok(Some(config)) = load_smtp_config_optional(&state).await else {
            return;
        };
        if send_message(Arc::new(config), recipient, subject, body)
            .await
            .is_err()
        {
            eprintln!("monitoring alert delivery failed");
        }
    });
}

/// Alert emails an account can turn on or off in its settings.
#[derive(Clone, Copy)]
pub enum AlertKind {
    Authenticity,
    Service,
    Metering,
    Availability,
}

impl AlertKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Authenticity => "authenticity",
            Self::Service => "service",
            Self::Metering => "metering",
            Self::Availability => "availability",
        }
    }

    /// Whether the kind is emailed when the account never saved a choice.
    fn default_enabled(self) -> bool {
        matches!(self, Self::Availability)
    }
}

/// Emails a monitoring alert to the account when it has that kind turned on.
pub async fn account_alert(
    state: &AppState,
    account_id: Uuid,
    kind: AlertKind,
    subject: &'static str,
    body: String,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT COALESCE((a.alert_emails->>$2)::boolean,$3) AS enabled,
           (SELECT u.email FROM memberships m JOIN users u ON u.id=m.user_id
             WHERE m.account_id=a.id ORDER BY m.created_at LIMIT 1) AS email
         FROM accounts a WHERE a.id=$1",
    )
    .bind(account_id)
    .bind(kind.key())
    .bind(kind.default_enabled())
    .fetch_optional(&state.db)
    .await?;
    if let Some(row) = row
        && row.get::<bool, _>("enabled")
        && let Some(recipient) = row.get::<Option<String>, _>("email")
    {
        alert_in_background(state, recipient, subject, body);
    }
    Ok(())
}

pub enum SecurityNotice {
    EmailChanged { new_email: String },
    PasswordChanged,
}

pub async fn send_test_email(config: Arc<SmtpConfig>, recipient: String) -> Result<()> {
    send_message(
        config,
        recipient,
        "SMTP 测试邮件",
        format!(
            "这是一封 {PRODUCT} SMTP 测试邮件。\n如果你能收到这封邮件，说明邮箱发送配置已生效。"
        ),
    )
    .await
}
