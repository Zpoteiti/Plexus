//! Server-only constants.
//!
//! These values are referenced only within `plexus-server`. They are kept here
//! (rather than in `plexus-common`) so the shared crate only holds constants
//! that cross the server/client/gateway boundary.

pub const TOOL_EXECUTION_TIMEOUT_SEC: u64 = 120;
pub const USER_MESSAGE_MAX_CHARS: usize = 4000;
pub const CONTEXT_COMPRESSION_THRESHOLD: usize = 16_000;
pub const WEB_FETCH_MAX_BODY_BYTES: usize = 1_048_576;
pub const WEB_FETCH_MAX_OUTPUT_CHARS: usize = 50_000;
pub const WEB_FETCH_TIMEOUT_SEC: u64 = 15;
pub const WEB_FETCH_CONNECT_TIMEOUT_SEC: u64 = 10;
pub const WEB_FETCH_MAX_REDIRECTS: usize = 5;
pub const WEB_FETCH_CONCURRENT_MAX: usize = 50;
pub const DB_POOL_MAX_CONNECTIONS: u32 = 200;
pub const RATE_LIMIT_CACHE_TTL_SEC: u64 = 60;
pub const JWT_EXPIRY_DAYS: i64 = 7;
pub const BCRYPT_COST: u32 = 12;
pub const HEARTBEAT_REAPER_INTERVAL_SEC: u64 = 30;
pub const CRON_POLL_INTERVAL_SEC: u64 = 10;
pub const MAX_UNCOMPRESSED_MESSAGES: i64 = 2000;
