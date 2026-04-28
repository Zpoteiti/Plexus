//! Integration test for ADR-104's "never log secrets" guarantee.
//!
//! Constructs structs holding secret newtypes (DeviceToken/JwtSecret/LlmApiKey)
//! and asserts that `format!("{:?}", x)` and `format!("{}", x)` never
//! reveal the inner value, even when the secret is nested inside another struct.

use plexus_common::secrets::{DeviceToken, JwtSecret, LlmApiKey};

const SECRET_LITERAL: &str = "this-is-a-secret-value-that-must-not-leak";
const TOKEN_LITERAL: &str = "plexus_dev_actualsecrettoken12345";

#[test]
fn device_token_debug_does_not_leak() {
    let t = DeviceToken::new(TOKEN_LITERAL.into());
    let dbg = format!("{:?}", t);
    assert!(!dbg.contains("actualsecrettoken"), "Debug leaked: {}", dbg);
}

#[test]
fn device_token_display_does_not_leak() {
    let t = DeviceToken::new(TOKEN_LITERAL.into());
    let disp = format!("{}", t);
    assert!(
        !disp.contains("actualsecrettoken"),
        "Display leaked: {}",
        disp
    );
}

#[test]
fn jwt_secret_does_not_leak_through_debug() {
    let j = JwtSecret::new(SECRET_LITERAL.into());
    let dbg = format!("{:?}", j);
    assert!(!dbg.contains("must-not-leak"), "Debug leaked: {}", dbg);
}

#[test]
fn llm_api_key_does_not_leak_through_debug() {
    let k = LlmApiKey::new(SECRET_LITERAL.into());
    let dbg = format!("{:?}", k);
    assert!(!dbg.contains("must-not-leak"), "Debug leaked: {}", dbg);
}

#[test]
fn secret_inside_struct_is_redacted() {
    #[derive(Debug)]
    #[allow(dead_code)] // fields are read via Debug derive; clippy ignores that
    struct DeviceConfig {
        name: String,
        token: DeviceToken,
    }
    let cfg = DeviceConfig {
        name: "mac-mini".into(),
        token: DeviceToken::new(TOKEN_LITERAL.into()),
    };
    let dbg = format!("{:?}", cfg);
    assert!(
        !dbg.contains("actualsecrettoken"),
        "Debug leaked through containing struct: {}",
        dbg
    );
    assert!(
        dbg.contains("mac-mini"),
        "non-secret field should still print"
    );
}

#[test]
fn expose_secret_returns_inner() {
    // Sanity check: the secret IS recoverable via the explicit API.
    let t = DeviceToken::new(TOKEN_LITERAL.into());
    assert_eq!(t.expose_secret(), TOKEN_LITERAL);
}

#[test]
fn cloning_secret_preserves_redaction() {
    let t = DeviceToken::new(TOKEN_LITERAL.into());
    let cloned = t.clone();
    let dbg_orig = format!("{:?}", t);
    let dbg_clone = format!("{:?}", cloned);
    assert!(!dbg_orig.contains("actualsecrettoken"));
    assert!(!dbg_clone.contains("actualsecrettoken"));
    assert_eq!(cloned.expose_secret(), TOKEN_LITERAL);
}
