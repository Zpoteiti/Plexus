//! Reserved string constants used across the protocol and tool layers.
//!
//! These are wire-level conventions — changing any of them is a breaking
//! protocol change.

/// Template for the untrusted-message wrap (ADR-007).
///
/// When a channel adapter receives a message from a non-partner, it prepends
/// this string (with the sender's display name substituted) to the content.
/// The `{}` is the sender name — adapters call `format!()` with it.
pub const UNTRUSTED_MESSAGE_PREFIX_TEMPLATE: &str = "[untrusted message from {}]: ";

/// Prefix wrapped onto every tool result before the LLM sees it (ADR-095).
pub const UNTRUSTED_TOOL_RESULT_PREFIX: &str = "[untrusted tool result]: ";

/// Reserved prefix for fields the merger injects into tool schemas (ADR-071).
///
/// MCP authors must not use this prefix on their own schema fields.
pub const PLEXUS_FIELD_PREFIX: &str = "plexus_";

/// Marker the merger sets on routing-only schema fields (ADR-071).
///
/// Used to distinguish merge-time-injected fields from intrinsic-device
/// fields when the schema is later modified.
pub const PLEXUS_DEVICE_MARKER: &str = "x-plexus-device";

/// Prefix for device tokens (ADR-091, ADR-097).
///
/// The full token format is `plexus_dev_<random base64 hash>`.
pub const DEVICE_TOKEN_PREFIX: &str = "plexus_dev_";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_message_prefix_format() {
        assert_eq!(
            UNTRUSTED_MESSAGE_PREFIX_TEMPLATE,
            "[untrusted message from {}]: "
        );
    }

    #[test]
    fn untrusted_tool_result_prefix() {
        assert_eq!(UNTRUSTED_TOOL_RESULT_PREFIX, "[untrusted tool result]: ");
    }

    #[test]
    fn plexus_field_prefix() {
        assert_eq!(PLEXUS_FIELD_PREFIX, "plexus_");
    }

    #[test]
    fn plexus_device_marker() {
        assert_eq!(PLEXUS_DEVICE_MARKER, "x-plexus-device");
    }

    #[test]
    fn device_token_prefix() {
        assert_eq!(DEVICE_TOKEN_PREFIX, "plexus_dev_");
    }
}
