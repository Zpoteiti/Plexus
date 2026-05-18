use crate::error::ApiError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
pub use plexus_common::{ContentBlock, ImageUrlBlock};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn parse_content_array(raw: &Value) -> Result<Vec<ContentBlock>, ApiError> {
    let Value::Array(values) = raw else {
        return Err(ApiError::invalid_args("content must be an array"));
    };
    values.iter().cloned().map(parse_block).collect()
}

fn parse_block(value: Value) -> Result<ContentBlock, ApiError> {
    let block: ContentBlock = serde_json::from_value(value)
        .map_err(|_| ApiError::invalid_args("content block is malformed"))?;
    if let ContentBlock::ImageUrl { image_url } = &block {
        decode_data_image_url(&image_url.url)?;
    }
    Ok(block)
}

pub fn decode_data_image_url(url: &str) -> Result<(String, Vec<u8>), ApiError> {
    let Some(rest) = url.strip_prefix("data:image/") else {
        return Err(ApiError::invalid_args(
            "image_url.url must be an inline data:image/...;base64 URL",
        ));
    };
    let Some((mime_tail, data)) = rest.split_once(";base64,") else {
        return Err(ApiError::invalid_args(
            "image_url.url must be a base64 data URL",
        ));
    };
    if mime_tail.is_empty()
        || !mime_tail
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    {
        return Err(ApiError::invalid_args(
            "image_url.url must be a valid data:image/...;base64 URL",
        ));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| ApiError::invalid_args("image_url.url base64 is invalid"))?;
    Ok((format!("image/{mime_tail}"), bytes))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
