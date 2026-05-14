use crate::error::ApiError;
pub use plexus_common::{ContentBlock, ImageUrlBlock};
use serde_json::Value;

pub fn normalize_user_content(raw: Option<Value>) -> Result<Vec<ContentBlock>, ApiError> {
    match raw {
        None => Ok(Vec::new()),
        Some(Value::String(text)) if text.is_empty() => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![ContentBlock::text(text)]),
        Some(Value::Array(values)) => values.into_iter().map(parse_block).collect(),
        Some(Value::Null) => Err(ApiError::invalid_args("content must not be null")),
        Some(_) => Err(ApiError::invalid_args("content must be a string or array")),
    }
}

fn parse_block(value: Value) -> Result<ContentBlock, ApiError> {
    let block: ContentBlock = serde_json::from_value(value)
        .map_err(|_| ApiError::invalid_args("content block is malformed"))?;
    validate_block(&block)?;
    Ok(block)
}

fn validate_block(block: &ContentBlock) -> Result<(), ApiError> {
    if let ContentBlock::ImageUrl { image_url } = block {
        validate_data_image_url(&image_url.url)?;
    }
    Ok(())
}

fn validate_data_image_url(url: &str) -> Result<(), ApiError> {
    let Some(rest) = url.strip_prefix("data:image/") else {
        return Err(ApiError::invalid_args(
            "M1c only accepts inline data:image/...;base64 image URLs",
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
        || data.is_empty()
        || !data
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '='))
    {
        return Err(ApiError::invalid_args(
            "image_url.url must be a valid data:image/...;base64 URL",
        ));
    }
    Ok(())
}
