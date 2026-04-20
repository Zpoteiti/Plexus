// User account info
export interface User {
  user_id: string
  email: string
  is_admin: boolean
  display_name: string | null
  created_at: string
}

// Session (conversation thread)
export interface Session {
  session_id: string
  user_id: string
  channel: string
  created_at: string
  updated_at: string
  hasUnread: boolean
}

// --- Anthropic-style content blocks (spec §2.1) ---
//
// User messages with attachments carry an array of content blocks. Assistant
// messages (today) come back as a plain string from the server; the client
// stores both shapes on `ChatMessage.content`.

export interface TextBlock {
  type: 'text'
  text: string
}

export interface ImageBlockSource {
  type: 'base64'
  media_type: string
  data: string
}

export interface ImageBlock {
  type: 'image'
  source: ImageBlockSource
  /** Non-standard, plexus-specific: workspace-relative path where the file lives. */
  workspace_path?: string
  /** Optional filename for alt text / downloads. */
  filename?: string
}

export type ContentBlock = TextBlock | ImageBlock

// Single chat message (user or assistant) — client-side representation.
// `content` may be a plain string (assistant replies, legacy user text) or an
// array of content blocks (user messages with image attachments).
export interface ChatMessage {
  id: string          // client UUID (optimistic) or server message_id
  session_id: string
  role: 'user' | 'assistant'
  content: string | ContentBlock[]
  created_at: string
}

// Server-side API message shape (from GET /api/sessions/{id}/messages)
export interface ApiMessage {
  message_id: string
  session_id: string
  role: 'user' | 'assistant' | 'tool'
  content: string
  tool_name: string | null
  tool_call_id: string | null
  created_at: string
}

// Connected client device
export interface Device {
  device_name: string
  status: 'online' | 'offline'
  tools_count: number
  fs_policy: { mode: 'sandbox' | 'unrestricted' }
}

// Device auth token
export interface DeviceToken {
  token: string
  device_name: string
  created_at: string
  last_used: string | null
}

// Per-device config (GET /api/devices/{name}/config response)
export interface DeviceConfig {
  device_name: string
  fs_policy: { mode: 'sandbox' | 'unrestricted' }
  workspace_path: string
  shell_timeout_max: number
  ssrf_whitelist: string[]
  mcp_servers: McpServerEntry[]
}

// MCP server config entry (used by both server and client MCP)
export interface McpServerEntry {
  name: string
  command: string
  args: string[]
  env?: Record<string, string>
  url?: string
  enabled: boolean
  tool_timeout?: number
}

// Discord channel config
export interface DiscordConfig {
  user_id: string
  bot_user_id: string
  enabled: boolean
  partner_discord_id: string
  allowed_users: string[]
}

// Telegram channel config
export interface TelegramConfig {
  partner_telegram_id: string
  allowed_users: string[]
  group_policy: 'mention' | 'all'
}

// LLM provider config (admin only)
export interface LlmConfig {
  api_base: string
  model: string
  api_key: string
  context_window: number
}

// Cron job
export interface CronJob {
  job_id: string
  user_id: string
  message: string
  name: string | null
  enabled: boolean
  cron_expr: string | null
  every_seconds: number | null
  at: string | null
  timezone: string | null
  channel: string | null
  created_at: string
}

// User skill
export interface Skill {
  name: string
  description: string
  always_on: boolean
  created_at: string
}

// Rate limit config (admin only)
export interface RateLimit {
  rate_limit_per_min: number
}

// Default soul config (admin only)
export interface DefaultSoul {
  soul: string
}

export type WorkspaceFile = {
  path: string;
  is_dir: boolean;
  size_bytes: number;
  modified_at: string;
};

export type WorkspaceQuota = {
  used_bytes: number;
  total_bytes: number;
};

export type WorkspaceSkill = {
  name: string;
  description: string;
  always_on: boolean;
};

// Admin user summary (from GET /api/admin/users)
export type AdminUser = {
  user_id: string;
  email: string;
  display_name: string | null;
  is_admin: boolean;
  created_at: string;
  last_heartbeat_at: string | null;
};
