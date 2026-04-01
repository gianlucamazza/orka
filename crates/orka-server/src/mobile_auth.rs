//! Mobile pairing and session issuance for the product-facing app.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use deadpool_redis::{Config as RedisConfig, Pool, Runtime, redis::Script};
use jsonwebtoken::{EncodingKey, Header, encode};
use rand::TryRngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

const MOBILE_SCOPE: &str = "mobile:chat";

/// Service errors for mobile auth and first-device pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileAuthError {
    /// Pairing/refresh is unavailable because token signing is not configured.
    Disabled,
    /// The caller provided invalid or malformed input.
    InvalidRequest(String),
    /// The requested pairing session does not exist.
    NotFound,
    /// The requested pairing session belongs to another authenticated caller.
    Forbidden,
    /// The pairing session or refresh token has expired.
    Expired,
    /// The pairing session was already completed.
    AlreadyUsed,
    /// The provided secret/token does not match a valid record.
    Unauthorized,
    /// Unexpected infrastructure failure.
    Internal(String),
}

impl fmt::Display for MobileAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "mobile pairing is unavailable on this server"),
            Self::InvalidRequest(message) | Self::Internal(message) => write!(f, "{message}"),
            Self::NotFound | Self::Forbidden => write!(f, "pairing session not found"),
            Self::Expired => write!(f, "pairing session has expired"),
            Self::AlreadyUsed => write!(f, "pairing session has already been used"),
            Self::Unauthorized => write!(f, "invalid pairing or refresh token"),
        }
    }
}

impl std::error::Error for MobileAuthError {}

/// Configuration for issuing Orka-managed mobile sessions.
#[derive(Debug, Clone)]
pub struct MobileAuthConfig {
    /// JWT issuer.
    pub issuer: String,
    /// Optional JWT audience.
    pub audience: Option<String>,
    /// HMAC signing secret.
    pub signing_secret: String,
    /// Access token lifetime.
    pub access_token_ttl: Duration,
    /// Refresh token lifetime.
    pub refresh_token_ttl: Duration,
    /// Pairing lifetime.
    pub pairing_ttl: Duration,
}

impl MobileAuthConfig {
    /// Build the v1 mobile auth configuration from signing inputs.
    pub fn new(issuer: String, audience: Option<String>, signing_secret: String) -> Self {
        Self {
            issuer,
            audience,
            signing_secret,
            access_token_ttl: Duration::minutes(15),
            refresh_token_ttl: Duration::days(30),
            pairing_ttl: Duration::minutes(5),
        }
    }
}

/// Pairing lifecycle visible to CLI polling.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    /// Ready to be completed by the mobile device.
    Pending,
    /// Successfully consumed and turned into a mobile session.
    Completed,
    /// No longer usable.
    Expired,
}

impl PairingStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Expired => "expired",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "completed" => Some(Self::Completed),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct PairingRecord {
    creator_principal: String,
    _server_base_url: String,
    secret_hash: String,
    status: PairingStatus,
    _created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    device_label: Option<String>,
}

#[derive(Debug, Clone)]
struct RefreshRecord {
    session_id: String,
    user_id: String,
    device_id: String,
    device_name: String,
    platform: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    _last_rotated_at: DateTime<Utc>,
}

/// Result returned to the CLI when a pairing session is created.
#[derive(Debug, Clone)]
pub struct CreatedPairing {
    /// Stable pairing identifier used by both CLI polling and mobile
    /// completion.
    pub pairing_id: String,
    /// One-time secret carried inside the QR/deep link and validated on
    /// completion.
    pub pairing_secret: String,
    /// Absolute expiration time for the one-time pairing session.
    pub expires_at: DateTime<Utc>,
    /// Deep-link URI encoded into the QR code for the mobile app.
    pub pairing_uri: String,
}

/// Pairing status view for CLI polling.
#[derive(Debug, Clone)]
pub struct PairingStatusView {
    /// Stable pairing identifier created by the server.
    pub pairing_id: String,
    /// Current lifecycle status of the pairing session.
    pub status: PairingStatus,
    /// Absolute expiration time for the pairing session.
    pub expires_at: DateTime<Utc>,
    /// Completion timestamp when the mobile device has successfully consumed
    /// the pairing.
    pub completed_at: Option<DateTime<Utc>>,
    /// Human-readable device label recorded at completion time.
    pub device_label: Option<String>,
}

/// Request payload used by the mobile app to finish pairing.
#[derive(Debug, Clone)]
pub struct CompletePairingInput {
    /// One-time pairing identifier embedded in the deep link.
    pub pairing_id: String,
    /// One-time pairing secret embedded in the deep link.
    pub pairing_secret: String,
    /// Stable per-installation device identifier generated by the app.
    pub device_id: String,
    /// Human-readable device name reported by the mobile OS.
    pub device_name: String,
    /// Platform label such as `ios` or `android`.
    pub platform: String,
}

/// Request payload used by the app to rotate mobile credentials.
#[derive(Debug, Clone)]
pub struct RefreshInput {
    /// Opaque refresh token previously issued by Orka.
    pub refresh_token: String,
    /// Stable per-installation device identifier used to bind refresh rotation.
    pub device_id: String,
}

/// Access and refresh material returned to the mobile app.
#[derive(Debug, Clone)]
pub struct MobileSession {
    /// Short-lived JWT used for authenticated mobile API requests.
    pub access_token: String,
    /// Absolute expiration time for the access token.
    pub access_token_expires_at: DateTime<Utc>,
    /// Opaque refresh token stored securely on the device.
    pub refresh_token: String,
    /// Absolute expiration time for the refresh token.
    pub refresh_token_expires_at: DateTime<Utc>,
    /// Authenticated user identifier bound to the mobile session.
    pub user_id: String,
}

/// Abstraction behind the public mobile auth routes.
#[async_trait]
pub trait MobileAuthService: Send + Sync {
    /// Create a one-time pairing session for a CLI-authenticated caller.
    async fn create_pairing(
        &self,
        creator_principal: &str,
        server_base_url: &str,
    ) -> Result<CreatedPairing, MobileAuthError>;

    /// Load the status of an existing pairing session for the same creator.
    async fn get_pairing_status(
        &self,
        creator_principal: &str,
        pairing_id: &str,
    ) -> Result<Option<PairingStatusView>, MobileAuthError>;

    /// Complete a one-time pairing and issue the first mobile device session.
    async fn complete_pairing(
        &self,
        input: CompletePairingInput,
    ) -> Result<MobileSession, MobileAuthError>;

    /// Rotate an existing refresh token and mint a new access token.
    async fn refresh_session(&self, input: RefreshInput) -> Result<MobileSession, MobileAuthError>;
}

/// In-memory implementation used by integration tests.
pub struct InMemoryMobileAuthService {
    config: MobileAuthConfig,
    pairings: Arc<Mutex<HashMap<String, PairingRecord>>>,
    refresh_tokens: Arc<Mutex<HashMap<String, RefreshRecord>>>,
}

impl InMemoryMobileAuthService {
    /// Create a new in-memory service.
    pub fn new(config: MobileAuthConfig) -> Self {
        Self {
            config,
            pairings: Arc::new(Mutex::new(HashMap::new())),
            refresh_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl MobileAuthService for InMemoryMobileAuthService {
    async fn create_pairing(
        &self,
        creator_principal: &str,
        server_base_url: &str,
    ) -> Result<CreatedPairing, MobileAuthError> {
        let server_base_url = normalize_server_base_url(server_base_url)?;
        let pairing_id = Uuid::now_v7().to_string();
        let pairing_secret = random_secret()?;
        let now = Utc::now();
        let expires_at = now + self.config.pairing_ttl;
        let record = PairingRecord {
            creator_principal: creator_principal.to_string(),
            _server_base_url: server_base_url.clone(),
            secret_hash: hash_secret(&pairing_secret),
            status: PairingStatus::Pending,
            _created_at: now,
            expires_at,
            completed_at: None,
            device_label: None,
        };

        self.pairings
            .lock()
            .await
            .insert(pairing_id.clone(), record);

        Ok(CreatedPairing {
            pairing_uri: build_pairing_uri(&server_base_url, &pairing_id, &pairing_secret)?,
            pairing_id,
            pairing_secret,
            expires_at,
        })
    }

    async fn get_pairing_status(
        &self,
        creator_principal: &str,
        pairing_id: &str,
    ) -> Result<Option<PairingStatusView>, MobileAuthError> {
        let mut pairings = self.pairings.lock().await;
        let Some(record) = pairings.get_mut(pairing_id) else {
            return Ok(None);
        };
        if record.creator_principal != creator_principal {
            return Err(MobileAuthError::Forbidden);
        }
        expire_pairing_if_needed(record);
        Ok(Some(PairingStatusView {
            pairing_id: pairing_id.to_string(),
            status: record.status,
            expires_at: record.expires_at,
            completed_at: record.completed_at,
            device_label: record.device_label.clone(),
        }))
    }

    async fn complete_pairing(
        &self,
        input: CompletePairingInput,
    ) -> Result<MobileSession, MobileAuthError> {
        validate_device_fields(&input.device_id, &input.device_name, &input.platform)?;

        let user_id = {
            let mut pairings = self.pairings.lock().await;
            let record = pairings
                .get_mut(&input.pairing_id)
                .ok_or(MobileAuthError::NotFound)?;
            expire_pairing_if_needed(record);
            match record.status {
                PairingStatus::Pending => {}
                PairingStatus::Completed => return Err(MobileAuthError::AlreadyUsed),
                PairingStatus::Expired => return Err(MobileAuthError::Expired),
            }
            if record.secret_hash != hash_secret(&input.pairing_secret) {
                return Err(MobileAuthError::Unauthorized);
            }
            record.status = PairingStatus::Completed;
            record.completed_at = Some(Utc::now());
            record.device_label = Some(device_label(&input.device_name, &input.platform));
            record.creator_principal.clone()
        };

        let session = issue_mobile_session(&self.config, &user_id, &input.device_id)?;
        self.refresh_tokens.lock().await.insert(
            hash_secret(&session.refresh_token),
            RefreshRecord {
                session_id: Uuid::now_v7().to_string(),
                user_id: user_id.clone(),
                device_id: input.device_id,
                device_name: input.device_name,
                platform: input.platform,
                expires_at: session.refresh_token_expires_at,
                created_at: Utc::now(),
                _last_rotated_at: Utc::now(),
            },
        );
        Ok(session)
    }

    async fn refresh_session(&self, input: RefreshInput) -> Result<MobileSession, MobileAuthError> {
        if input.refresh_token.trim().is_empty() {
            return Err(MobileAuthError::InvalidRequest(
                "refresh_token must not be empty".to_string(),
            ));
        }
        if input.device_id.trim().is_empty() {
            return Err(MobileAuthError::InvalidRequest(
                "device_id must not be empty".to_string(),
            ));
        }

        let record = {
            let mut refresh_tokens = self.refresh_tokens.lock().await;
            let key = hash_secret(&input.refresh_token);
            let record = refresh_tokens
                .remove(&key)
                .ok_or(MobileAuthError::Unauthorized)?;
            if record.device_id != input.device_id {
                return Err(MobileAuthError::Unauthorized);
            }
            if record.expires_at <= Utc::now() {
                return Err(MobileAuthError::Expired);
            }
            record
        };

        let session = issue_mobile_session(&self.config, &record.user_id, &record.device_id)?;
        self.refresh_tokens.lock().await.insert(
            hash_secret(&session.refresh_token),
            RefreshRecord {
                session_id: record.session_id,
                user_id: record.user_id,
                device_id: record.device_id,
                device_name: record.device_name,
                platform: record.platform,
                expires_at: session.refresh_token_expires_at,
                created_at: record.created_at,
                _last_rotated_at: Utc::now(),
            },
        );
        Ok(session)
    }
}

/// Redis-backed implementation used by the server runtime.
pub struct RedisMobileAuthService {
    config: MobileAuthConfig,
    pool: Pool,
}

impl RedisMobileAuthService {
    /// Create a new Redis-backed service.
    pub fn new(redis_url: &str, config: MobileAuthConfig) -> Result<Self, MobileAuthError> {
        let pool = RedisConfig::from_url(redis_url)
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|error| {
                MobileAuthError::Internal(format!(
                    "failed to create mobile auth Redis pool: {error}"
                ))
            })?;
        Ok(Self { config, pool })
    }
}

#[async_trait]
impl MobileAuthService for RedisMobileAuthService {
    async fn create_pairing(
        &self,
        creator_principal: &str,
        server_base_url: &str,
    ) -> Result<CreatedPairing, MobileAuthError> {
        let server_base_url = normalize_server_base_url(server_base_url)?;
        let pairing_id = Uuid::now_v7().to_string();
        let pairing_secret = random_secret()?;
        let now = Utc::now();
        let expires_at = now + self.config.pairing_ttl;
        let key = pairing_key(&pairing_id);
        let mut conn = self.pool.get().await.map_err(internal_redis_error)?;

        redis::pipe()
            .atomic()
            .cmd("HSET")
            .arg(&key)
            .arg("creator_principal")
            .arg(creator_principal)
            .arg("server_base_url")
            .arg(&server_base_url)
            .arg("secret_hash")
            .arg(hash_secret(&pairing_secret))
            .arg("status")
            .arg(PairingStatus::Pending.as_str())
            .arg("created_at")
            .arg(now.timestamp())
            .arg("expires_at")
            .arg(expires_at.timestamp())
            .ignore()
            .cmd("EXPIRE")
            .arg(&key)
            .arg(self.config.pairing_ttl.num_seconds())
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .map_err(internal_redis_error)?;

        Ok(CreatedPairing {
            pairing_uri: build_pairing_uri(&server_base_url, &pairing_id, &pairing_secret)?,
            pairing_id,
            pairing_secret,
            expires_at,
        })
    }

    async fn get_pairing_status(
        &self,
        creator_principal: &str,
        pairing_id: &str,
    ) -> Result<Option<PairingStatusView>, MobileAuthError> {
        let key = pairing_key(pairing_id);
        let mut conn = self.pool.get().await.map_err(internal_redis_error)?;
        let data: HashMap<String, String> = redis::cmd("HGETALL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(internal_redis_error)?;
        if data.is_empty() {
            return Ok(None);
        }

        let record = pairing_record_from_map(&data)?;
        if record.creator_principal != creator_principal {
            return Err(MobileAuthError::Forbidden);
        }

        let mut status = record.status;
        if status == PairingStatus::Pending && record.expires_at <= Utc::now() {
            status = PairingStatus::Expired;
            let _: Result<(), _> = redis::cmd("HSET")
                .arg(&key)
                .arg("status")
                .arg(PairingStatus::Expired.as_str())
                .query_async(&mut conn)
                .await;
        }

        Ok(Some(PairingStatusView {
            pairing_id: pairing_id.to_string(),
            status,
            expires_at: record.expires_at,
            completed_at: record.completed_at,
            device_label: record.device_label,
        }))
    }

    async fn complete_pairing(
        &self,
        input: CompletePairingInput,
    ) -> Result<MobileSession, MobileAuthError> {
        validate_device_fields(&input.device_id, &input.device_name, &input.platform)?;

        let key = pairing_key(&input.pairing_id);
        let secret_hash = hash_secret(&input.pairing_secret);
        let now = Utc::now().timestamp();
        let label = device_label(&input.device_name, &input.platform);
        let mut conn = self.pool.get().await.map_err(internal_redis_error)?;
        let result = Script::new(
            r"
local key = KEYS[1]
local expected_hash = ARGV[1]
local now_ts = tonumber(ARGV[2])
local device_label = ARGV[3]
if redis.call('EXISTS', key) == 0 then
  return {'not_found'}
end
local status = redis.call('HGET', key, 'status')
if status == 'completed' then
  return {'completed'}
end
local expires_at = tonumber(redis.call('HGET', key, 'expires_at'))
if expires_at == nil or expires_at <= now_ts then
  redis.call('HSET', key, 'status', 'expired')
  return {'expired'}
end
local actual_hash = redis.call('HGET', key, 'secret_hash')
if actual_hash ~= expected_hash then
  return {'unauthorized'}
end
redis.call('HSET', key, 'status', 'completed', 'completed_at', now_ts, 'device_label', device_label)
return {'ok', redis.call('HGET', key, 'creator_principal')}
",
        )
        .key(&key)
        .arg(secret_hash)
        .arg(now)
        .arg(label)
        .invoke_async::<Vec<String>>(&mut conn)
        .await
        .map_err(internal_redis_error)?;

        match result.first().map(String::as_str) {
            Some("ok") => {}
            Some("not_found") => return Err(MobileAuthError::NotFound),
            Some("completed") => return Err(MobileAuthError::AlreadyUsed),
            Some("expired") => return Err(MobileAuthError::Expired),
            Some("unauthorized") => return Err(MobileAuthError::Unauthorized),
            _ => {
                return Err(MobileAuthError::Internal(
                    "unexpected Redis pairing response".to_string(),
                ));
            }
        }

        let user_id = result
            .get(1)
            .cloned()
            .ok_or_else(|| MobileAuthError::Internal("missing pairing principal".to_string()))?;
        let session = issue_mobile_session(&self.config, &user_id, &input.device_id)?;
        store_refresh_record(
            &mut conn,
            &session,
            &input.device_id,
            &input.device_name,
            &input.platform,
            self.config.refresh_token_ttl,
        )
        .await?;
        Ok(session)
    }

    async fn refresh_session(&self, input: RefreshInput) -> Result<MobileSession, MobileAuthError> {
        if input.refresh_token.trim().is_empty() {
            return Err(MobileAuthError::InvalidRequest(
                "refresh_token must not be empty".to_string(),
            ));
        }
        if input.device_id.trim().is_empty() {
            return Err(MobileAuthError::InvalidRequest(
                "device_id must not be empty".to_string(),
            ));
        }

        let key = refresh_key(&input.refresh_token);
        let mut conn = self.pool.get().await.map_err(internal_redis_error)?;
        let result = Script::new(
            r"
local key = KEYS[1]
local expected_device_id = ARGV[1]
local now_ts = tonumber(ARGV[2])
if redis.call('EXISTS', key) == 0 then
  return {'missing'}
end
local device_id = redis.call('HGET', key, 'device_id')
if device_id ~= expected_device_id then
  return {'device_mismatch'}
end
local expires_at = tonumber(redis.call('HGET', key, 'expires_at'))
if expires_at == nil or expires_at <= now_ts then
  redis.call('DEL', key)
  return {'expired'}
end
local user_id = redis.call('HGET', key, 'user_id')
local session_id = redis.call('HGET', key, 'session_id')
local device_name = redis.call('HGET', key, 'device_name')
local platform = redis.call('HGET', key, 'platform')
local created_at = redis.call('HGET', key, 'created_at')
redis.call('DEL', key)
return {'ok', user_id, session_id, device_name, platform, created_at}
",
        )
        .key(&key)
        .arg(input.device_id.trim())
        .arg(Utc::now().timestamp())
        .invoke_async::<Vec<String>>(&mut conn)
        .await
        .map_err(internal_redis_error)?;

        match result.first().map(String::as_str) {
            Some("ok") => {}
            Some("missing" | "device_mismatch") => {
                return Err(MobileAuthError::Unauthorized);
            }
            Some("expired") => return Err(MobileAuthError::Expired),
            _ => {
                return Err(MobileAuthError::Internal(
                    "unexpected Redis refresh response".to_string(),
                ));
            }
        }

        let user_id = result
            .get(1)
            .cloned()
            .ok_or_else(|| MobileAuthError::Internal("missing refresh user".to_string()))?;
        let session_id = result
            .get(2)
            .cloned()
            .ok_or_else(|| MobileAuthError::Internal("missing refresh session id".to_string()))?;
        let device_name = result
            .get(3)
            .cloned()
            .unwrap_or_else(|| "Mobile device".to_string());
        let platform = result
            .get(4)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let created_at = result
            .get(5)
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(|ts| DateTime::from_timestamp(ts, 0))
            .unwrap_or_else(Utc::now);

        let session = issue_mobile_session(&self.config, &user_id, &input.device_id)?;
        store_refresh_record_with_session_id(
            &mut conn,
            &session,
            &input.device_id,
            &device_name,
            &platform,
            created_at,
            &session_id,
            self.config.refresh_token_ttl,
        )
        .await?;
        Ok(session)
    }
}

fn internal_redis_error(error: impl fmt::Display) -> MobileAuthError {
    MobileAuthError::Internal(format!("mobile auth Redis error: {error}"))
}

fn pairing_key(pairing_id: &str) -> String {
    format!("orka:mobile:pairing:{pairing_id}")
}

fn refresh_key(refresh_token: &str) -> String {
    format!("orka:mobile:refresh:{}", hash_secret(refresh_token))
}

fn device_label(device_name: &str, platform: &str) -> String {
    let device_name = device_name.trim();
    let platform = platform.trim();
    if platform.is_empty() || platform.eq_ignore_ascii_case(device_name) {
        device_name.to_string()
    } else {
        format!("{device_name} ({platform})")
    }
}

fn hash_secret(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

fn random_secret() -> Result<String, MobileAuthError> {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|error| {
            MobileAuthError::Internal(format!("failed to generate secure random bytes: {error}"))
        })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn normalize_server_base_url(value: &str) -> Result<String, MobileAuthError> {
    let mut url = Url::parse(value.trim()).map_err(|_| {
        MobileAuthError::InvalidRequest("server_base_url must be a valid URL".to_string())
    })?;
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(MobileAuthError::InvalidRequest(
                "server_base_url must use http or https".to_string(),
            ));
        }
    }
    if url.host_str().is_none() {
        return Err(MobileAuthError::InvalidRequest(
            "server_base_url must include a host".to_string(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn build_pairing_uri(
    server_base_url: &str,
    pairing_id: &str,
    pairing_secret: &str,
) -> Result<String, MobileAuthError> {
    let uri = Url::parse_with_params(
        "mobileorka://pair",
        [
            ("server", server_base_url),
            ("pairing_id", pairing_id),
            ("pairing_secret", pairing_secret),
        ],
    )
    .map_err(|error| MobileAuthError::Internal(format!("failed to build pairing URI: {error}")))?;
    Ok(uri.to_string())
}

fn validate_device_fields(
    device_id: &str,
    device_name: &str,
    platform: &str,
) -> Result<(), MobileAuthError> {
    if device_id.trim().is_empty() {
        return Err(MobileAuthError::InvalidRequest(
            "device_id must not be empty".to_string(),
        ));
    }
    if device_name.trim().is_empty() {
        return Err(MobileAuthError::InvalidRequest(
            "device_name must not be empty".to_string(),
        ));
    }
    if platform.trim().is_empty() {
        return Err(MobileAuthError::InvalidRequest(
            "platform must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn expire_pairing_if_needed(record: &mut PairingRecord) {
    if record.status == PairingStatus::Pending && record.expires_at <= Utc::now() {
        record.status = PairingStatus::Expired;
    }
}

#[derive(Serialize)]
struct MobileClaims<'a> {
    sub: &'a str,
    scope: &'a str,
    iss: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<&'a str>,
    exp: u64,
    iat: u64,
}

fn issue_mobile_session(
    config: &MobileAuthConfig,
    user_id: &str,
    device_id: &str,
) -> Result<MobileSession, MobileAuthError> {
    let access_now = Utc::now();
    let access_expires_at = access_now + config.access_token_ttl;
    let refresh_expires_at = access_now + config.refresh_token_ttl;
    let claims = MobileClaims {
        sub: user_id,
        scope: MOBILE_SCOPE,
        iss: &config.issuer,
        aud: config.audience.as_deref(),
        exp: access_expires_at.timestamp() as u64,
        iat: access_now.timestamp() as u64,
    };
    let access_token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.signing_secret.as_bytes()),
    )
    .map_err(|error| MobileAuthError::Internal(format!("failed to issue access token: {error}")))?;

    let refresh_token = format!("{device_id}.{}", random_secret()?);

    Ok(MobileSession {
        access_token,
        access_token_expires_at: access_expires_at,
        refresh_token,
        refresh_token_expires_at: refresh_expires_at,
        user_id: user_id.to_string(),
    })
}

async fn store_refresh_record(
    conn: &mut deadpool_redis::Connection,
    session: &MobileSession,
    device_id: &str,
    device_name: &str,
    platform: &str,
    refresh_token_ttl: Duration,
) -> Result<(), MobileAuthError> {
    store_refresh_record_with_session_id(
        conn,
        session,
        device_id,
        device_name,
        platform,
        Utc::now(),
        &Uuid::now_v7().to_string(),
        refresh_token_ttl,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn store_refresh_record_with_session_id(
    conn: &mut deadpool_redis::Connection,
    session: &MobileSession,
    device_id: &str,
    device_name: &str,
    platform: &str,
    created_at: DateTime<Utc>,
    session_id: &str,
    refresh_token_ttl: Duration,
) -> Result<(), MobileAuthError> {
    let key = refresh_key(&session.refresh_token);
    redis::pipe()
        .atomic()
        .cmd("HSET")
        .arg(&key)
        .arg("session_id")
        .arg(session_id)
        .arg("user_id")
        .arg(&session.user_id)
        .arg("device_id")
        .arg(device_id)
        .arg("device_name")
        .arg(device_name)
        .arg("platform")
        .arg(platform)
        .arg("created_at")
        .arg(created_at.timestamp())
        .arg("last_rotated_at")
        .arg(Utc::now().timestamp())
        .arg("expires_at")
        .arg(session.refresh_token_expires_at.timestamp())
        .ignore()
        .cmd("EXPIRE")
        .arg(&key)
        .arg(refresh_token_ttl.num_seconds())
        .ignore()
        .query_async::<()>(conn)
        .await
        .map_err(internal_redis_error)?;
    Ok(())
}

fn pairing_record_from_map(
    data: &HashMap<String, String>,
) -> Result<PairingRecord, MobileAuthError> {
    let creator_principal = data
        .get("creator_principal")
        .cloned()
        .ok_or_else(|| MobileAuthError::Internal("pairing creator missing".to_string()))?;
    let server_base_url = data
        .get("server_base_url")
        .cloned()
        .ok_or_else(|| MobileAuthError::Internal("pairing base URL missing".to_string()))?;
    let secret_hash = data
        .get("secret_hash")
        .cloned()
        .ok_or_else(|| MobileAuthError::Internal("pairing secret hash missing".to_string()))?;
    let status = data
        .get("status")
        .and_then(|value| PairingStatus::parse(value))
        .ok_or_else(|| MobileAuthError::Internal("pairing status missing".to_string()))?;
    let created_at = data
        .get("created_at")
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .ok_or_else(|| MobileAuthError::Internal("pairing created_at missing".to_string()))?;
    let expires_at = data
        .get("expires_at")
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .ok_or_else(|| MobileAuthError::Internal("pairing expires_at missing".to_string()))?;
    let completed_at = data
        .get("completed_at")
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|ts| DateTime::from_timestamp(ts, 0));

    Ok(PairingRecord {
        creator_principal,
        _server_base_url: server_base_url,
        secret_hash,
        status,
        _created_at: created_at,
        expires_at,
        completed_at,
        device_label: data.get("device_label").cloned(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_pairing_round_trip_and_refresh_rotation() {
        let service = InMemoryMobileAuthService::new(MobileAuthConfig::new(
            "orka-tests".to_string(),
            None,
            "test-secret-key-at-least-32-bytes-long!".to_string(),
        ));

        let created = service
            .create_pairing("operator-1", "https://orka.example.com")
            .await
            .unwrap();
        let status = service
            .get_pairing_status("operator-1", &created.pairing_id)
            .await
            .unwrap()
            .expect("pairing should exist");
        assert_eq!(status.status, PairingStatus::Pending);

        let session = service
            .complete_pairing(CompletePairingInput {
                pairing_id: created.pairing_id.clone(),
                pairing_secret: created.pairing_secret.clone(),
                device_id: "device-1".to_string(),
                device_name: "Pixel".to_string(),
                platform: "android".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(session.user_id, "operator-1");

        let rotated = service
            .refresh_session(RefreshInput {
                refresh_token: session.refresh_token.clone(),
                device_id: "device-1".to_string(),
            })
            .await
            .unwrap();
        assert_ne!(rotated.refresh_token, session.refresh_token);

        let reused = service
            .refresh_session(RefreshInput {
                refresh_token: session.refresh_token,
                device_id: "device-1".to_string(),
            })
            .await;
        assert!(matches!(reused, Err(MobileAuthError::Unauthorized)));
    }
}
