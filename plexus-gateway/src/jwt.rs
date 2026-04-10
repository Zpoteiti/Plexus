use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: u64,
}

/// Validate a JWT and return the claims.
pub fn validate(token: &str, secret: &str) -> Result<Claims, String> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["sub", "exp"]);

    decode::<Claims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| format!("JWT validation failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const SECRET: &str = "test-secret-key";

    fn make_token(sub: &str, is_admin: bool, exp: u64) -> String {
        let claims = Claims {
            sub: sub.to_string(),
            is_admin,
            exp,
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(SECRET.as_bytes()))
            .unwrap()
    }

    fn future_exp() -> u64 {
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs())
            + 3600
    }

    #[test]
    fn valid_token() {
        let token = make_token("user42", false, future_exp());
        let claims = validate(&token, SECRET).unwrap();
        assert_eq!(claims.sub, "user42");
        assert!(!claims.is_admin);
    }

    #[test]
    fn expired_token() {
        let token = make_token("user42", false, 1000);
        let result = validate(&token, SECRET);
        assert!(result.is_err(), "Expected expired token to fail validation");
    }

    #[test]
    fn wrong_secret() {
        let token = make_token("user42", false, future_exp());
        let result = validate(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn malformed_token() {
        let result = validate("not.a.jwt", SECRET);
        assert!(result.is_err());
    }

    #[test]
    fn completely_garbage() {
        let result = validate("garbage", SECRET);
        assert!(result.is_err());
    }
}
