use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub database_url: String,
    pub workspace_root: PathBuf,
    pub bind: SocketAddr,
    pub jwt_secret: String,
    pub admin_token: Option<String>,
    pub cookie_secure: bool,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, env::VarError> {
        let database_url = env::var("DATABASE_URL")?;
        let workspace_root = PathBuf::from(env::var("PLEXUS_WORKSPACE_ROOT")?);
        let bind = env::var("PLEXUS_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .expect("PLEXUS_BIND must be host:port");
        let jwt_secret = env::var("JWT_SECRET")?;
        let admin_token = env::var("ADMIN_TOKEN").ok();
        let cookie_secure = env::var("PLEXUS_COOKIE_SECURE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

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
