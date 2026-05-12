use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

const JWT_TTL_SECONDS: i64 = 60 * 60 * 24 * 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub is_admin: bool,
    pub iat: i64,
    pub exp: i64,
}

pub fn issue_token(
    secret: &str,
    user_id: Uuid,
    is_admin: bool,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = Claims {
        sub: user_id,
        is_admin,
        iat: now,
        exp: now + JWT_TTL_SECONDS,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub fn verify_token(secret: &str, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "plexus_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        token, JWT_TTL_SECONDS
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_session_cookie(secure: bool) -> String {
    let mut cookie = "plexus_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0".to_string();
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn issued_token_verifies() {
        let user_id = Uuid::now_v7();
        let token = issue_token("secret", user_id, true).unwrap();
        let claims = verify_token("secret", &token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert!(claims.is_admin);
    }
}
