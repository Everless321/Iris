use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::{request::Parts, StatusCode};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const JWT_TTL_SECS: u64 = 7 * 24 * 3600; // 7 天

#[derive(Clone)]
pub struct AuthState {
    pub enc: EncodingKey,
    pub dec: DecodingKey,
}

impl AuthState {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            enc: EncodingKey::from_secret(secret),
            dec: DecodingKey::from_secret(secret),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i64,        // user id
    pub username: String,
    pub role: String,    // admin | customer
    pub exp: u64,
}

impl Claims {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

pub fn hash_password(pw: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?
        .to_string())
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .and_then(|h| Argon2::default().verify_password(pw.as_bytes(), &h))
        .is_ok()
}

pub fn issue_token(state: &AuthState, user_id: i64, username: &str, role: &str) -> Result<String> {
    let exp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + JWT_TTL_SECS;
    let claims = Claims {
        sub: user_id,
        username: username.into(),
        role: role.into(),
        exp,
    };
    Ok(encode(&Header::default(), &claims, &state.enc)?)
}

pub fn verify_token(state: &AuthState, token: &str) -> Result<Claims> {
    Ok(decode::<Claims>(token, &state.dec, &Validation::default())?.claims)
}

/// Axum 提取器：Authorization: Bearer <jwt> → Claims。
#[axum::async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_state = AuthState::from_ref(state);
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".into()))?;
        verify_token(&auth_state, token)
            .map_err(|e| (StatusCode::UNAUTHORIZED, format!("invalid token: {e}")))
    }
}

/// 仅 admin 可调用的提取器。除签名/过期外，再查 DB 确认用户存在且当前 role=admin。
/// 防止用户被删除或降级后旧 token 仍可访问 admin 端点。
pub struct AdminClaims(pub Claims);

#[axum::async_trait]
impl<S> FromRequestParts<S> for AdminClaims
where
    S: Send + Sync,
    AuthState: FromRef<S>,
    sqlx::SqlitePool: FromRef<S>,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let c = Claims::from_request_parts(parts, state).await?;
        if !c.is_admin() {
            return Err((StatusCode::FORBIDDEN, "admin only".into()));
        }
        let pool = sqlx::SqlitePool::from_ref(state);
        let role: Option<String> =
            sqlx::query_scalar("SELECT role FROM users WHERE id=?")
                .bind(c.sub)
                .fetch_optional(&pool)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "auth check failed".into()))?;
        match role.as_deref() {
            Some("admin") => Ok(AdminClaims(c)),
            Some(_) => Err((StatusCode::FORBIDDEN, "role downgraded".into())),
            None => Err((StatusCode::UNAUTHORIZED, "user removed".into())),
        }
    }
}
