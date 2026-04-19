//! Telegram per-user bot channel via teloxide.
//! Long polling. Group @mention detection. Access control via allowed_users.

use crate::bus::{self, EventKind, InboundEvent, OutboundEvent};
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatKind;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const TELEGRAM_MSG_LIMIT: usize = 4096;

/// Telegram's native attachment size cap. Files above this are rejected
/// before we even try to download or upload.
const TELEGRAM_ATTACHMENT_MAX_BYTES: usize = 20 * 1024 * 1024;

type BotRegistry = Arc<RwLock<HashMap<String, BotHandle>>>;

struct BotHandle {
    /// teloxide bot instance for outbound delivery
    bot: Arc<RwLock<Option<Bot>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

static BOT_REGISTRY: std::sync::LazyLock<BotRegistry> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Start a Telegram bot for a user.
pub async fn start_bot(state: Arc<AppState>, user_id: String, bot_token: String) {
    stop_bot(&user_id).await;

    let bot_holder: Arc<RwLock<Option<Bot>>> = Arc::new(RwLock::new(None));
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

    BOT_REGISTRY.write().await.insert(
        user_id.clone(),
        BotHandle {
            bot: Arc::clone(&bot_holder),
            shutdown_tx: Some(shutdown_tx),
        },
    );

    // Load telegram config for access control
    let config = crate::db::telegram::get_config(&state.db, &user_id)
        .await
        .ok()
        .flatten();
    let partner_telegram_id = config
        .as_ref()
        .and_then(|c| c.partner_telegram_id.clone())
        .unwrap_or_default();
    let allowed_users: Vec<String> = config
        .as_ref()
        .map(|c| c.allowed_users.clone())
        .unwrap_or_default();
    let group_policy = config
        .as_ref()
        .map(|c| c.group_policy.clone())
        .unwrap_or_else(|| "mention".into());

    let state_clone = Arc::clone(&state);
    let state_shutdown = Arc::clone(&state);
    let bot_holder_clone = Arc::clone(&bot_holder);

    tokio::spawn(async move {
        info!("Telegram bot starting for user {user_id}");

        let bot = Bot::new(&bot_token);
        *bot_holder_clone.write().await = Some(bot.clone());

        // Get bot username for mention detection
        let bot_username = match bot.get_me().await {
            Ok(me) => me.username.clone().unwrap_or_default(),
            Err(e) => {
                error!("Telegram getMe failed for {user_id}: {e}");
                return;
            }
        };
        info!("Telegram bot ready: @{bot_username} for user {user_id}");

        let user_id_log = user_id.clone();
        let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
            let state = Arc::clone(&state_clone);
            let plexus_user_id = user_id.clone();
            let owner_id = partner_telegram_id.clone();
            let allowed = allowed_users.clone();
            let policy = group_policy.clone();
            let username = bot_username.clone();

            async move {
                handle_message(
                    &state,
                    &bot,
                    &msg,
                    &plexus_user_id,
                    &owner_id,
                    &allowed,
                    &policy,
                    &username,
                )
                .await;
                respond(())
            }
        });

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .enable_ctrlc_handler()
            .build();

        tokio::select! {
            _ = dispatcher.dispatch() => {
                info!("Telegram bot stopped for {}", &user_id_log);
            }
            _ = &mut shutdown_rx => {
                info!("Telegram bot shutdown requested for {}", &user_id_log);
                match dispatcher.shutdown_token().shutdown() {
                    Ok(fut) => fut.await,
                    Err(e) => warn!("Telegram shutdown failed: {e}"),
                }
            }
            _ = state_shutdown.shutdown.cancelled() => {
                info!(user_id = %user_id_log, "Telegram bot shutting down");
                match dispatcher.shutdown_token().shutdown() {
                    Ok(fut) => fut.await,
                    Err(e) => warn!("Telegram shutdown failed: {e}"),
                }
            }
        }
    });
}

async fn handle_message(
    state: &Arc<AppState>,
    bot: &Bot,
    msg: &Message,
    plexus_user_id: &str,
    partner_telegram_id: &str,
    allowed_users: &[String],
    group_policy: &str,
    bot_username: &str,
) {
    // Extract text content (empty if the message is media-only).
    let text = msg.text().unwrap_or("").to_string();

    // Get sender info
    let sender = match msg.from.as_ref() {
        Some(u) => u,
        None => return,
    };
    let sender_id = sender.id.0.to_string();
    let sender_name = sender.first_name.clone();

    // Access control: partner or allowed_users
    let is_partner = sender_id == partner_telegram_id;
    if !is_partner && !allowed_users.contains(&sender_id) {
        return;
    }

    // Group check: only respond when @mentioned (if policy is "mention")
    let is_group = matches!(msg.chat.kind, ChatKind::Public(_));
    if is_group && group_policy == "mention" {
        let mention_tag = format!("@{bot_username}");
        if !text.contains(&mention_tag) {
            // Also check if it's a reply to the bot
            let is_reply_to_bot = msg
                .reply_to_message()
                .and_then(|r| r.from.as_ref())
                .map(|u| u.is_bot)
                .unwrap_or(false);
            if !is_reply_to_bot {
                return;
            }
        }
    }

    // Strip @mention from content
    let mut content = text
        .replace(&format!("@{bot_username}"), "")
        .trim()
        .to_string();

    // Harvest inbound attachments into the file store.
    let now = chrono::Utc::now();
    let mut media_urls: Vec<String> = Vec::new();

    let attachments: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = Vec::new();
        if let Some(photos) = msg.photo() {
            if let Some(largest) = photos.last() {
                v.push((largest.file.id.clone(), synth_filename_for_photo(now)));
            }
        }
        if let Some(voice) = msg.voice() {
            v.push((voice.file.id.clone(), synth_filename_for_voice(now)));
        }
        if let Some(audio) = msg.audio() {
            let name = audio
                .title
                .clone()
                .unwrap_or_else(|| synth_filename_for_audio(now));
            v.push((audio.file.id.clone(), name));
        }
        if let Some(document) = msg.document() {
            let name = document
                .file_name
                .clone()
                .unwrap_or_else(|| format!("document_{}", now.format("%Y%m%d_%H%M%S")));
            v.push((document.file.id.clone(), name));
        }
        if let Some(video) = msg.video() {
            let name = video
                .file_name
                .clone()
                .unwrap_or_else(|| synth_filename_for_video(now));
            v.push((video.file.id.clone(), name));
        }
        if let Some(video_note) = msg.video_note() {
            v.push((
                video_note.file.id.clone(),
                synth_filename_for_video_note(now),
            ));
        }
        if let Some(animation) = msg.animation() {
            let name = animation
                .file_name
                .clone()
                .unwrap_or_else(|| synth_filename_for_animation(now));
            v.push((animation.file.id.clone(), name));
        }
        v
    };

    for (file_id, filename) in attachments {
        let file = match bot.get_file(&file_id).await {
            Ok(f) => f,
            Err(e) => {
                warn!("telegram getFile failed ({filename}): {e}");
                content.push('\n');
                content.push_str(&format!("[Attachment: {filename} — download failed]"));
                continue;
            }
        };

        if (file.size as usize) > TELEGRAM_ATTACHMENT_MAX_BYTES {
            content.push('\n');
            content.push_str(&oversize_attachment_marker(&filename, file.size as u64));
            continue;
        }

        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot.token(),
            file.path
        );

        let bytes = match state.http_client.get(&download_url).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    warn!("telegram file read failed ({filename}): {e}");
                    content.push('\n');
                    content.push_str(&format!("[Attachment: {filename} — download failed]"));
                    continue;
                }
            },
            Err(e) => {
                warn!("telegram file fetch failed ({filename}): {e}");
                content.push('\n');
                content.push_str(&format!("[Attachment: {filename} — download failed]"));
                continue;
            }
        };

        let safe_name = filename.replace(['/', '\\'], "_");
        let rel = format!(".attachments/telegram-{}/{}", msg.id, safe_name);
        match state.workspace_fs.write(plexus_user_id, &rel, &bytes).await {
            Ok(()) => media_urls.push(rel),
            Err(e) => {
                warn!("telegram attachment save failed ({filename}): {e}");
                content.push('\n');
                content.push_str(&format!("[Attachment: {filename} — storage failed]"));
            }
        }
    }

    if content.is_empty() && media_urls.is_empty() {
        return;
    }

    let chat_id = format!("tg:{}", msg.chat.id);
    let session_id = format!("telegram:{}", msg.chat.id);

    let event = InboundEvent {
        session_id,
        user_id: plexus_user_id.to_string(),
        kind: EventKind::UserTurn,
        content,
        channel: crate::channels::CHANNEL_TELEGRAM.to_string(),
        chat_id: Some(chat_id),
        media: media_urls,
        cron_job_id: None,
        identity: Some(crate::context::ChannelIdentity {
            sender_name: sender_name.clone(),
            sender_id,
            is_partner,
            channel_type: crate::channels::CHANNEL_TELEGRAM.to_string(),
        }),
    };

    if let Err(e) = bus::publish_inbound(state, event).await {
        error!("Telegram inbound error: {e}");
    }
}

/// Stop a Telegram bot for a user.
pub async fn stop_bot(user_id: &str) {
    if let Some(mut handle) = BOT_REGISTRY.write().await.remove(user_id) {
        if let Some(tx) = handle.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Deliver an outbound event via Telegram.
pub async fn deliver(state: &Arc<AppState>, event: &OutboundEvent) {
    let registry = BOT_REGISTRY.read().await;
    let handle = match registry.get(&event.user_id) {
        Some(h) => h,
        None => {
            warn!("Telegram: no bot for user {}", event.user_id);
            return;
        }
    };

    let bot_lock = handle.bot.read().await;
    let Some(bot) = bot_lock.as_ref() else {
        warn!("Telegram: bot not ready for user {}", event.user_id);
        return;
    };
    let bot = bot.clone();

    // Parse chat_id (format: "tg:{chat_id}")
    let chat_id_str = match event.chat_id.as_deref() {
        Some(id) => id.strip_prefix("tg:").unwrap_or(id),
        None => {
            warn!("Telegram: no chat_id for outbound");
            return;
        }
    };
    let chat_id: ChatId = match chat_id_str.parse::<i64>() {
        Ok(id) => ChatId(id),
        Err(_) => {
            warn!("Telegram: invalid chat_id: {chat_id_str}");
            return;
        }
    };

    // Split and send message
    let chunks = split_message(&event.content, TELEGRAM_MSG_LIMIT);
    for chunk in &chunks {
        if let Err(e) = bot.send_message(chat_id, chunk).await {
            error!("Telegram send error: {e}");
        }
    }

    // Send media as file attachments via workspace_fs
    for path in &event.media {
        let bytes = match state.workspace_fs.read(&event.user_id, path).await {
            Ok(b) => b,
            Err(e) => {
                warn!("Telegram media read failed ({path}): {e}");
                continue;
            }
        };
        let filename = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file.bin".into());
        let input = teloxide::types::InputFile::memory(bytes).file_name(filename);
        if let Err(e) = bot.send_document(chat_id, input).await {
            error!("Telegram media send error: {e}");
        }
    }
}

fn split_message(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= limit {
            chunks.push(remaining.to_string());
            break;
        }
        let split_at = remaining[..limit].rfind('\n').unwrap_or(limit);
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }
    chunks
}

fn synth_filename_for_voice(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("voice_message_{}.ogg", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_photo(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("photo_{}.jpg", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_video(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("video_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_audio(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("audio_{}.mp3", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_animation(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("animation_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_video_note(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("video_note_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}

/// Inline marker for an attachment that exceeds the upload size limit.
fn oversize_attachment_marker(name: &str, size: u64) -> String {
    format!(
        "[Attachment: {name} ({:.1} MB) — exceeds {} MB limit, not downloaded]",
        size as f64 / 1024.0 / 1024.0,
        TELEGRAM_ATTACHMENT_MAX_BYTES / 1024 / 1024
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_synth_filename_voice() {
        let ts = Utc.with_ymd_and_hms(2026, 4, 15, 10, 30, 5).unwrap();
        assert_eq!(
            synth_filename_for_voice(ts),
            "voice_message_20260415_103005.ogg"
        );
    }

    #[test]
    fn test_synth_filename_photo() {
        let ts = Utc.with_ymd_and_hms(2026, 4, 15, 10, 30, 5).unwrap();
        assert_eq!(synth_filename_for_photo(ts), "photo_20260415_103005.jpg");
    }

    #[test]
    fn test_oversize_attachment_marker() {
        let marker =
            oversize_attachment_marker("big.zip", (TELEGRAM_ATTACHMENT_MAX_BYTES + 1) as u64);
        assert!(marker.contains("big.zip"));
        assert!(marker.contains("exceeds"));
        assert!(marker.contains("20 MB"));
    }
}
