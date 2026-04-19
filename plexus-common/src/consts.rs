//! Shared constants between Server and Client.
//! Prevents hardcoded magic numbers or strings on either side.

pub const PROTOCOL_VERSION: &str = "1.0";
pub const HEARTBEAT_INTERVAL_SEC: u64 = 15;
pub const DEFAULT_MCP_TOOL_TIMEOUT_SEC: u64 = 30;
pub const MAX_AGENT_ITERATIONS: u32 = 200;
pub const MAX_TOOL_OUTPUT_CHARS: usize = 10_000;
pub const TOOL_OUTPUT_HEAD_CHARS: usize = 5_000;
pub const TOOL_OUTPUT_TAIL_CHARS: usize = 5_000;

pub const EXIT_CODE_SUCCESS: i32 = 0;
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_TIMEOUT: i32 = -1;
pub const EXIT_CODE_CANCELLED: i32 = -2;

pub const DEVICE_TOKEN_PREFIX: &str = "plexus_dev_";

pub const SERVER_DEVICE_NAME: &str = "server";

// File tool limits
pub const MAX_READ_FILE_CHARS: usize = 128_000;
pub const DEFAULT_READ_FILE_LIMIT: usize = 2000;
pub const DEFAULT_LIST_DIR_MAX: usize = 200;

// Message roles (prevents typos in stringly-typed matching)
pub const ROLE_SYSTEM: &str = "system";
pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";
pub const ROLE_TOOL: &str = "tool";

// Channel names
pub const CHANNEL_GATEWAY: &str = "gateway";
pub const CHANNEL_DISCORD: &str = "discord";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(PROTOCOL_VERSION, "1.0");
        assert_eq!(HEARTBEAT_INTERVAL_SEC, 15);
        assert_eq!(
            MAX_TOOL_OUTPUT_CHARS,
            TOOL_OUTPUT_HEAD_CHARS + TOOL_OUTPUT_TAIL_CHARS
        );
        assert_eq!(EXIT_CODE_SUCCESS, 0);
        assert_eq!(DEVICE_TOKEN_PREFIX, "plexus_dev_");
    }

    #[test]
    fn test_file_tool_constants() {
        assert_eq!(MAX_READ_FILE_CHARS, 128_000);
        assert_eq!(DEFAULT_READ_FILE_LIMIT, 2000);
        assert_eq!(DEFAULT_LIST_DIR_MAX, 200);
    }
}
