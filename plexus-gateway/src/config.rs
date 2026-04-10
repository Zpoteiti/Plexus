/// Gateway configuration loaded from environment variables.

#[derive(Debug, Clone)]
pub struct Config {
    pub gateway_token: String,
    pub jwt_secret: String,
    pub port: u16,
    pub server_api_url: String,
    pub frontend_dir: String,
    pub allowed_origins: AllowedOrigins,
}

#[derive(Debug, Clone)]
pub enum AllowedOrigins {
    Any,
    List(Vec<String>),
}

impl Config {
    /// Load config from environment. Panics on missing required vars.
    pub fn from_env() -> Self {
        Self {
            gateway_token: std::env::var("PLEXUS_GATEWAY_TOKEN")
                .expect("PLEXUS_GATEWAY_TOKEN required"),
            jwt_secret: std::env::var("JWT_SECRET").expect("JWT_SECRET required"),
            port: std::env::var("GATEWAY_PORT")
                .expect("GATEWAY_PORT required")
                .parse()
                .expect("GATEWAY_PORT must be a number"),
            server_api_url: std::env::var("PLEXUS_SERVER_API_URL")
                .expect("PLEXUS_SERVER_API_URL required"),
            frontend_dir: std::env::var("PLEXUS_FRONTEND_DIR")
                .unwrap_or_else(|_| "../plexus-frontend/dist".into()),
            allowed_origins: match std::env::var("PLEXUS_ALLOWED_ORIGINS")
                .unwrap_or_else(|_| "*".into())
                .as_str()
            {
                "*" => AllowedOrigins::Any,
                list => AllowedOrigins::List(
                    list.split(',').map(|s| s.trim().to_string()).collect(),
                ),
            },
        }
    }

    /// Check if the given origin is allowed.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        match &self.allowed_origins {
            AllowedOrigins::Any => true,
            AllowedOrigins::List(list) => match origin {
                Some(o) => list.iter().any(|allowed| allowed == o),
                None => false, // strict mode requires an origin
            },
        }
    }
}
