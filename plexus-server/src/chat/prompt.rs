use crate::{
    db::{sessions::Session, users::User},
    error::ApiError,
};
use std::path::{Path, PathBuf};

pub async fn build_system_prompt(
    workspace_root: &Path,
    user: &User,
    session: &Session,
) -> Result<String, ApiError> {
    let user_root = workspace_root.join(user.id.to_string());
    let soul = read_optional(user_root.join("SOUL.md")).await?;
    let memory = read_optional(user_root.join("MEMORY.md")).await?;

    Ok(format!(
        "## SOUL\n\n{soul}\n\n---\n\n\
         ## MEMORY\n\n{memory}\n\n---\n\n\
         ## Identity\n\n\
         You are Plexus, partnered with one human: {name} (account `{id}`).\n\
         Input typed directly by {name} in this browser chat is authoritative.\n\n---\n\n\
         ## Channels\n\n\
         Current channel: web\n\
         Current chat_id: {chat_id}\n\
         Direct replies go to this browser session.\n\n---\n\n\
         ## Operating Notes\n\n\
         M1c has no tools available. Answer in plain text. Do not claim access to files, devices, MCP, workspace tools, cron, Discord, Telegram, or message tools.",
        soul = soul,
        memory = memory,
        name = user.name,
        id = user.id,
        chat_id = session.chat_id,
    ))
}

async fn read_optional(path: PathBuf) -> Result<String, ApiError> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(ApiError::invalid_args(format!(
            "failed to read prompt file: {err}"
        ))),
    }
}
