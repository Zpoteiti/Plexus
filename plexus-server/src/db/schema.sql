CREATE TABLE IF NOT EXISTS users (
    id              TEXT        PRIMARY KEY,
    email           TEXT        NOT NULL UNIQUE,
    password_hash   TEXT        NOT NULL,
    display_name    TEXT        NOT NULL,
    timezone        TEXT        NOT NULL DEFAULT 'UTC',
    is_admin        BOOLEAN     NOT NULL DEFAULT FALSE,
    last_dream_at   TIMESTAMPTZ,
    last_heartbeat_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS devices (
    id                  TEXT        PRIMARY KEY,
    user_id             TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT        NOT NULL,
    token_hash          TEXT        NOT NULL,
    workspace_path      TEXT        NOT NULL,
    shell_timeout_max   INTEGER     NOT NULL DEFAULT 300,
    ssrf_whitelist      TEXT[]      NOT NULL DEFAULT '{}',
    fs_policy           TEXT        NOT NULL DEFAULT 'sandbox'
                         CHECK (fs_policy IN ('sandbox', 'unrestricted')),
    mcp_servers         JSONB       NOT NULL DEFAULT '[]',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at        TIMESTAMPTZ,
    UNIQUE (user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_devices_user_id ON devices(user_id);

CREATE TABLE IF NOT EXISTS device_tokens (
    token           TEXT        PRIMARY KEY,
    user_id         TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at     TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_device_tokens_user_id ON device_tokens(user_id);

CREATE TABLE IF NOT EXISTS sessions (
    id              TEXT        PRIMARY KEY,
    user_id         TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel         TEXT        NOT NULL,           -- "gateway" | "discord" | "tg:{chat_id}"
    title           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_activity   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, channel)
);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id_last_activity ON sessions(user_id, last_activity DESC);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT        PRIMARY KEY,
    session_id      TEXT        NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role            TEXT        NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content         JSONB       NOT NULL,           -- Anthropic-style content blocks
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_messages_session_id_created ON messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id                  TEXT        PRIMARY KEY,
    user_id             TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT        NOT NULL,
    kind                TEXT        NOT NULL DEFAULT 'user'
                         CHECK (kind IN ('user', 'system')),
    schedule            TEXT        NOT NULL,
    prompt              TEXT        NOT NULL,
    enabled             BOOLEAN     NOT NULL DEFAULT TRUE,
    last_run_at         TIMESTAMPTZ,
    next_run_at         TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_user_id ON cron_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run_at) WHERE enabled = TRUE;
CREATE UNIQUE INDEX IF NOT EXISTS idx_cron_jobs_system_per_user
    ON cron_jobs(user_id, name) WHERE kind = 'system';

CREATE TABLE IF NOT EXISTS discord_configs (
    user_id         TEXT        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token       TEXT        NOT NULL,
    channel_id      TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS telegram_configs (
    user_id         TEXT        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token       TEXT        NOT NULL,
    allowed_chat_ids TEXT[]     NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS system_config (
    key             TEXT        PRIMARY KEY,
    value           JSONB       NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
-- Seeded keys: llm_config, rate_limit, dream_phase1_prompt, dream_phase2_prompt,
-- heartbeat_phase1_prompt, server_mcp, workspace_quota_default_bytes, etc.
