//! Version constants. See ADR-107.
//!
//! The protocol version is the WS wire-protocol version. The binary release
//! version comes from `Cargo.toml`'s `version` field at compile time.

/// The current Plexus WebSocket protocol version (ADR-107).
///
/// Bumps only when the wire format changes in a breaking way. Most binary
/// releases do NOT bump this — see ADR-107 for the rules.
pub const PROTOCOL_VERSION: &str = "1";

/// Returns the running crate's version (e.g. `"0.1.0"`) as set in `Cargo.toml`.
///
/// Read at compile time via `env!("CARGO_PKG_VERSION")` — no runtime cost.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_v1() {
        assert_eq!(PROTOCOL_VERSION, "1");
    }

    #[test]
    fn crate_version_is_semver_shaped() {
        let v = crate_version();
        assert!(!v.is_empty());
        assert!(v.contains('.'), "expected SemVer-shaped, got {v}");
    }
}
