//! Newtypes for secret values, wrapping `secrecy::SecretString`.
//!
//! Each type's `Debug`/`Display` impls redact. Exposing the inner value
//! requires explicit `secrecy::ExposeSecret`. See ADR-104.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! secret_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone)]
        pub struct $name(SecretString);

        impl $name {
            /// Construct from a plain string. The string is moved into the
            /// secret and zeroized on drop.
            pub fn new(value: String) -> Self {
                Self(SecretString::new(value.into()))
            }

            /// Access the inner string. Use sparingly — every call is an
            /// audit point for secret leakage.
            pub fn expose_secret(&self) -> &str {
                self.0.expose_secret()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&"<redacted>")
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("<redacted>")
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self::new(s)
            }
        }

        // secrecy 0.10's `SecretString` (= `SecretBox<str>`) does not
        // implement `Serialize`/`Deserialize` automatically because `str` is
        // unsized and not `SerializableSecret`. We implement them by hand on
        // the newtype, going through the plain `String` representation.
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.expose_secret().serialize(s)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d).map(Self::new)
            }
        }
    };
}

secret_newtype!(
    DeviceToken,
    "Bearer token issued at device registration (ADR-091, ADR-097).\n\nFormat: `plexus_dev_<base64>`. The full literal value of `PLEXUS_DEVICE_TOKEN` env var on the client side."
);

secret_newtype!(
    JwtSecret,
    "Server-side JWT signing secret. Loaded from env at server startup."
);

secret_newtype!(
    LlmApiKey,
    "API key for the OpenAI-compatible LLM endpoint (ADR-101). Stored in `system_config.llm_api_key`."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_token_debug_redacts() {
        let token = DeviceToken::new("plexus_dev_abc123secret".into());
        let dbg = format!("{:?}", token);
        assert!(
            !dbg.contains("abc123secret"),
            "Debug leaked secret: {}",
            dbg
        );
    }

    #[test]
    fn device_token_display_redacts() {
        let token = DeviceToken::new("plexus_dev_abc123secret".into());
        let disp = format!("{}", token);
        assert!(
            !disp.contains("abc123secret"),
            "Display leaked secret: {}",
            disp
        );
    }

    #[test]
    fn device_token_expose_secret_returns_value() {
        let token = DeviceToken::new("plexus_dev_abc123secret".into());
        assert_eq!(token.expose_secret(), "plexus_dev_abc123secret");
    }

    #[test]
    fn jwt_secret_redacts() {
        let secret = JwtSecret::new("my-jwt-secret".into());
        let dbg = format!("{:?}", secret);
        assert!(!dbg.contains("jwt-secret"), "Debug leaked: {}", dbg);
    }

    #[test]
    fn llm_api_key_redacts() {
        let key = LlmApiKey::new("sk-proj-actualsecretkey".into());
        let dbg = format!("{:?}", key);
        assert!(!dbg.contains("actualsecretkey"), "Debug leaked: {}", dbg);
    }
}
