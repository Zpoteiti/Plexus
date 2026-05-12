use plexus_common::{AdminToken, JwtSecret};
use std::{env, net::SocketAddr, path::PathBuf};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub database_url: String,
    pub workspace_root: PathBuf,
    pub bind: SocketAddr,
    pub jwt_secret: JwtSecret,
    pub admin_token: Option<AdminToken>,
    pub cookie_secure: bool,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Env(#[from] env::VarError),
    #[error("{0} must not be empty")]
    EmptySecret(&'static str),
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = env::var("DATABASE_URL")?;
        let workspace_root = PathBuf::from(env::var("PLEXUS_WORKSPACE_ROOT")?);
        let bind = env::var("PLEXUS_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .expect("PLEXUS_BIND must be host:port");
        let jwt_secret = env::var("JWT_SECRET")?;
        let admin_token = match env::var("ADMIN_TOKEN") {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => None,
            Err(err) => return Err(err.into()),
        };
        let cookie_secure = env::var("PLEXUS_COOKIE_SECURE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        Self::from_values(
            database_url,
            workspace_root,
            bind,
            jwt_secret,
            admin_token,
            cookie_secure,
        )
    }

    fn from_values(
        database_url: String,
        workspace_root: PathBuf,
        bind: SocketAddr,
        jwt_secret: String,
        admin_token: Option<String>,
        cookie_secure: bool,
    ) -> Result<Self, ConfigError> {
        let jwt_secret = JwtSecret::new(non_empty_secret("JWT_SECRET", jwt_secret)?);
        let admin_token = admin_token
            .map(|token| non_empty_secret("ADMIN_TOKEN", token).map(AdminToken::new))
            .transpose()?;

        Ok(Self {
            database_url,
            workspace_root,
            bind,
            jwt_secret,
            admin_token,
            cookie_secure,
        })
    }
}

fn non_empty_secret(name: &'static str, value: String) -> Result<String, ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::EmptySecret(name));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_auth_secrets() {
        let cfg = ServerConfig {
            database_url: "postgres://plexus:plexus@127.0.0.1:5432/plexus".to_string(),
            workspace_root: PathBuf::from("/tmp/plexus-workspaces"),
            bind: "127.0.0.1:8080".parse().unwrap(),
            jwt_secret: JwtSecret::new("actual-jwt-secret".to_string()),
            admin_token: Some(AdminToken::new("actual-admin-token".to_string())),
            cookie_secure: true,
        };

        let debug = format!("{cfg:?}");
        assert!(!debug.contains("actual-jwt-secret"));
        assert!(!debug.contains("actual-admin-token"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn rejects_empty_auth_secrets() {
        let jwt_result = ServerConfig::from_values(
            "postgres://plexus:plexus@127.0.0.1:5432/plexus".to_string(),
            PathBuf::from("/tmp/plexus-workspaces"),
            "127.0.0.1:8080".parse().unwrap(),
            "   ".to_string(),
            None,
            false,
        );
        assert!(matches!(
            jwt_result,
            Err(ConfigError::EmptySecret("JWT_SECRET"))
        ));

        let admin_result = ServerConfig::from_values(
            "postgres://plexus:plexus@127.0.0.1:5432/plexus".to_string(),
            PathBuf::from("/tmp/plexus-workspaces"),
            "127.0.0.1:8080".parse().unwrap(),
            "actual-jwt-secret".to_string(),
            Some("   ".to_string()),
            false,
        );
        assert!(matches!(
            admin_result,
            Err(ConfigError::EmptySecret("ADMIN_TOKEN"))
        ));
    }
}
