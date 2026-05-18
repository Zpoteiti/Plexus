use crate::{
    chat::content::{decode_data_image_url, sha256_hex},
    error::ApiError,
    workspace::WorkspaceFs,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use plexus_common::{ContentBlock, ImageUrlBlock};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRef {
    pub plexus_device: String,
    pub path: String,
}

pub async fn assemble_user_content(
    workspace: &WorkspaceFs,
    user_id: Uuid,
    user_content: Vec<ContentBlock>,
    raw_attachments: &Value,
) -> Result<Vec<ContentBlock>, ApiError> {
    let attachments = parse_attachments(raw_attachments)?;
    let direct_hashes = direct_image_hashes(&user_content)?;
    let mut prefix = Vec::new();
    let mut markers_before_direct: BTreeMap<usize, Vec<ContentBlock>> = BTreeMap::new();

    for attachment in attachments {
        if attachment.plexus_device != "server" {
            return Err(ApiError::invalid_args(
                "attachment plexus_device must be 'server'",
            ));
        }
        if Path::new(&attachment.path).is_absolute() {
            return Err(ApiError::invalid_args(
                "attachment path must be relative to the workspace",
            ));
        }

        let bytes = workspace.read_file(user_id, &attachment.path).await?;
        let marker = ContentBlock::text(format!(
            "User uploaded file to device='server', path={:?}",
            attachment.path
        ));

        let Some(mime) = sniff_image_mime(&bytes) else {
            prefix.push(marker);
            continue;
        };

        let attachment_hash = sha256_hex(&bytes);
        if let Some((index, _)) = direct_hashes
            .iter()
            .find(|(_, direct_hash)| direct_hash == &attachment_hash)
        {
            markers_before_direct
                .entry(*index)
                .or_default()
                .push(marker);
        } else {
            prefix.push(marker);
            prefix.push(ContentBlock::ImageUrl {
                image_url: ImageUrlBlock {
                    url: format!("data:{mime};base64,{}", STANDARD.encode(&bytes)),
                },
            });
        }
    }

    let mut assembled = Vec::with_capacity(
        prefix.len()
            + user_content.len()
            + markers_before_direct.values().map(Vec::len).sum::<usize>(),
    );
    assembled.extend(prefix);
    for (index, block) in user_content.into_iter().enumerate() {
        if let Some(markers) = markers_before_direct.remove(&index) {
            assembled.extend(markers);
        }
        assembled.push(block);
    }
    Ok(assembled)
}

fn parse_attachments(raw_attachments: &Value) -> Result<Vec<AttachmentRef>, ApiError> {
    let Value::Array(values) = raw_attachments else {
        return Err(ApiError::invalid_args("attachments must be an array"));
    };
    values
        .iter()
        .cloned()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|_| ApiError::invalid_args("attachment is malformed"))
        })
        .collect()
}

fn direct_image_hashes(user_content: &[ContentBlock]) -> Result<Vec<(usize, String)>, ApiError> {
    user_content
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            ContentBlock::ImageUrl { image_url } => Some(
                decode_data_image_url(&image_url.url).map(|(_, bytes)| (index, sha256_hex(&bytes))),
            ),
            ContentBlock::Text { .. } => None,
        })
        .collect()
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP" {
        return Some("image/webp");
    }
    None
}
