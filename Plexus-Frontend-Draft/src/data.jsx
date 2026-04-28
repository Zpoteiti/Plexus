// Realistic Plexus data — devices, sessions, messages, etc.

const PLEXUS_DATA = {
  user: {
    name: "Alice Chen",
    email: "alice@example.dev",
    initials: "AC",
    is_admin: true,
    user_id: "u_4f9c2a1e",
  },

  devices: [
    {
      name: "alice-laptop",
      os: "darwin",
      online: true,
      last_seen: "now",
      workspace_path: "/Users/alice/.plexus/",
      fs_policy: "sandbox",
      shell_timeout_max: 300,
      ssrf_whitelist: ["10.180.20.30:8080", "internal.corp:443"],
      mcp_count: 3,
      caps: { sandbox: "sandbox-exec", exec: true, fs: "rw" },
      client_version: "0.3.0",
      tools_in_flight: 0,
      mcp_servers: [
        { name: "minimax", tools: 4, resources: 2, prompts: 1 },
        { name: "linear", tools: 7, resources: 0, prompts: 0 },
        { name: "github", tools: 12, resources: 3, prompts: 2 },
      ],
    },
    {
      name: "prod-runner",
      os: "linux",
      online: true,
      last_seen: "2s ago",
      workspace_path: "/var/lib/plexus/",
      fs_policy: "sandbox",
      shell_timeout_max: 600,
      ssrf_whitelist: [],
      mcp_count: 1,
      caps: { sandbox: "bwrap", exec: true, fs: "rw" },
      client_version: "0.3.0",
      tools_in_flight: 1,
      mcp_servers: [
        { name: "postgres-readonly", tools: 5, resources: 0, prompts: 0 },
      ],
    },
    {
      name: "dev-vm",
      os: "linux",
      online: true,
      last_seen: "now",
      workspace_path: "/home/dev/.plexus/",
      fs_policy: "unrestricted",
      shell_timeout_max: 300,
      ssrf_whitelist: ["10.0.0.0/8"],
      mcp_count: 0,
      caps: { sandbox: "bwrap", exec: true, fs: "rw" },
      client_version: "0.3.0",
      tools_in_flight: 0,
      mcp_servers: [],
    },
    {
      name: "alice-phone",
      os: "android",
      online: false,
      last_seen: "4h ago",
      workspace_path: "/storage/emulated/0/plexus/",
      fs_policy: "sandbox",
      shell_timeout_max: 60,
      ssrf_whitelist: [],
      mcp_count: 0,
      caps: { sandbox: "none", exec: false, fs: "rw" },
      client_version: "0.2.9",
      tools_in_flight: 0,
      mcp_servers: [],
    },
  ],

  workspaces: [
    {
      name: "personal",
      kind: "personal",
      path_segment: "u_4f9c2a1e",
      bytes_used: 1_842_000_000,
      quota_bytes: 5_000_000_000,
      members: 1,
      file_count: 4218,
      locked: false,
    },
    {
      name: "production_department",
      kind: "shared",
      path_segment: "production_department",
      bytes_used: 18_400_000_000,
      quota_bytes: 25_000_000_000,
      members: 6,
      file_count: 12_487,
      locked: false,
    },
    {
      name: "research_pool",
      kind: "shared",
      path_segment: "research_pool",
      bytes_used: 23_900_000_000,
      quota_bytes: 25_000_000_000,
      members: 4,
      file_count: 9_104,
      locked: false,
    },
  ],

  sessions: [
    {
      id: "s_8a4d",
      session_key: "web:alice:main",
      title: "Triage logs from prod-runner outage",
      channel: "web",
      last_at: "now",
      unread: false,
      cancel_requested: false,
      streaming: true,
      msg_count: 14,
    },
    {
      id: "s_72b1",
      session_key: "web:alice:repo-audit",
      title: "Audit DECISIONS.md against schema.sql",
      channel: "web",
      last_at: "12m",
      unread: false,
      msg_count: 38,
    },
    {
      id: "s_44c3",
      session_key: "discord:alice#0001:dm",
      title: "Quick note about workspace quotas",
      channel: "discord",
      last_at: "2h",
      unread: true,
      msg_count: 6,
    },
    {
      id: "s_19fa",
      session_key: "cron:cj_morning_brief",
      title: "Morning briefing — 09:00 PT",
      channel: "cron",
      last_at: "8h",
      msg_count: 4,
    },
    {
      id: "s_9023",
      session_key: "telegram:alice:dm",
      title: "Reminder to drink water",
      channel: "telegram",
      last_at: "yesterday",
      msg_count: 22,
    },
    {
      id: "s_5e0c",
      session_key: "web:alice:plex-rust-port",
      title: "Port nanobot's edit_file matcher to Rust",
      channel: "web",
      last_at: "yesterday",
      msg_count: 67,
    },
    {
      id: "s_3d11",
      session_key: "web:alice:cron-design",
      title: "Design review: ADR-053 cron channel inheritance",
      channel: "web",
      last_at: "2d",
      msg_count: 19,
    },
  ],

  // Messages for session s_8a4d
  messages: [
    { id: "m1", role: "user", text: "prod-runner went red around 14:02. Pull the last 200 lines of journalctl from the device and tell me what you see.", t: "14:08:03", channel: "web" },
    {
      id: "m2", role: "assistant",
      blocks: [
        { kind: "text", text: "Pulling logs from prod-runner now. I'll check the agent loop and the bwrap subprocess for anything fatal." },
      ],
      t: "14:08:04",
    },
    {
      id: "m3", role: "tool",
      tool: { name: "exec", device: "prod-runner", id: "0190d5a8-7a24" },
      args: { command: "journalctl -u plexus-client -n 200 --no-pager", timeout: 30 },
      result: {
        ok: true,
        bytes: 14_280,
        stdout: "Apr 27 14:01:58 prod-runner plexus-client[2841]: tool_call dispatched id=0190d5a7-fe19 name=exec\nApr 27 14:01:59 prod-runner plexus-client[2841]: bwrap: failed to mount /etc/resolv.conf (read-only file system)\nApr 27 14:01:59 prod-runner plexus-client[2841]: tool_result is_error=true code=sandbox_failure\nApr 27 14:02:14 prod-runner plexus-client[2841]: WS pong missed 1/2\nApr 27 14:02:44 prod-runner plexus-client[2841]: WS pong missed 2/2 — closing\nApr 27 14:02:44 prod-runner plexus-client[2841]: ws closed code=4408 reason=heartbeat_timeout",
      },
      t: "14:08:09",
    },
    {
      id: "m4", role: "assistant",
      blocks: [
        { kind: "text", text: "Found it. Two distinct things, in this order:" },
        { kind: "list", items: [
          "**14:01:59** — `bwrap` failed to mount `/etc/resolv.conf` read-only. The sandbox bailed before the tool could run, so the agent saw `code=sandbox_failure`. ADR-073's mount list assumes that path exists; on this box it doesn't (resolv.conf is a symlink to `/run/systemd/resolve/stub-resolv.conf`).",
          "**14:02:14–14:02:44** — Two `pong` deadlines were missed in a row. Server hit close-code `4408` and marked the device offline. Anything in flight got `device_unreachable`.",
        ]},
        { kind: "text", text: "The first one caused a cascade: the failing tool likely held the agent loop, the client process didn't service its read loop, and pongs piled up. Want me to (a) patch the bwrap mount list to use `--ro-bind-try` so missing paths don't kill the jail, or (b) just symlink resolv.conf on prod-runner and call it a day?" },
        { kind: "buttons", buttons: [["Patch bwrap mount list", "Symlink resolv.conf", "Both"]] },
      ],
      t: "14:08:11",
    },
    { id: "m5", role: "user", text: "Patch bwrap mount list — and add a heartbeat-self-test on next start.", t: "14:09:22" },
    {
      id: "m6", role: "tool",
      tool: { name: "read_file", device: "server", id: "0190d5aa-1133" },
      args: { path: "/u_4f9c2a1e/plexus/plexus-client/src/sandbox/bwrap.rs" },
      result: { ok: true, bytes: 6_140, stdout: "1| use std::process::Command;\n2| use crate::config::FsPolicy;\n3| ...\n  (Showing lines 1-200 of 247.)" },
      t: "14:09:23",
    },
    {
      id: "m7", role: "tool",
      tool: { name: "edit_file", device: "server", id: "0190d5aa-9d10" },
      args: { path: "/u_4f9c2a1e/plexus/plexus-client/src/sandbox/bwrap.rs", old_text: "--ro-bind", new_text: "--ro-bind-try", replace_all: true },
      result: { ok: true, replacements: 7, stdout: "Replaced 7 occurrences." },
      t: "14:09:24",
    },
    {
      id: "m8", role: "assistant",
      blocks: [
        { kind: "text", text: "Patch landed — 7 mount calls flipped from `--ro-bind` to `--ro-bind-try`. Missing paths will now log a warning instead of aborting the jail." },
        { kind: "code", lang: "diff", text:
`-cmd.arg("--ro-bind").arg("/etc/resolv.conf").arg("/etc/resolv.conf");
+cmd.arg("--ro-bind-try").arg("/etc/resolv.conf").arg("/etc/resolv.conf");` },
        { kind: "text", text: "Want me to add a startup self-test that fires a no-op `tool_call` against the local jail right after `hello_ack` and surfaces any mount failures before the device goes live?" },
      ],
      t: "14:09:26",
    },
  ],

  cron_jobs: [
    { id: "cj_001", name: "morning-brief", schedule: "0 9 * * *", tz: "America/Los_Angeles", description: "Pull overnight Sentry errors and summarize by service", channel: "discord", chat_id: "1234…", deliver: true, last_fired_at: "today 09:00", next_fire_at: "tomorrow 09:00", one_shot: false },
    { id: "cj_002", name: "weekly-quota-report", schedule: "0 8 * * 1", tz: "America/Los_Angeles", description: "Email per-user workspace quota usage", channel: "web", chat_id: "alice:main", deliver: false, last_fired_at: "Apr 21 08:00", next_fire_at: "Apr 28 08:00", one_shot: false },
    { id: "cj_003", name: "drink-water", schedule: "every 2700s", tz: null, description: "Send a friendly hydration reminder", channel: "telegram", chat_id: "447…", deliver: true, last_fired_at: "12m ago", next_fire_at: "in 33m", one_shot: false },
    { id: "cj_004", name: "remind-pr-review", schedule: "2026-04-28T15:00:00", tz: "America/Los_Angeles", description: "Remind me to finish reviewing PR #1287", channel: "web", chat_id: "alice:main", deliver: true, last_fired_at: null, next_fire_at: "Apr 28 15:00", one_shot: true },
  ],

  channels: {
    discord: { configured: true, partner_chat_id: "234…891", allow_list_count: 2, bot_token_masked: "MTQ4…••••" },
    telegram: { configured: true, partner_chat_id: "447…102", allow_list_count: 0, bot_token_masked: "73••…••XKp" },
  },

  admin: {
    config: {
      llm_endpoint: "https://api.openai.com/v1",
      llm_model: "gpt-4o",
      llm_max_context_tokens: 128_000,
      llm_compaction_threshold_tokens: 16_000,
      quota_bytes: 5_000_000_000,
      shared_workspace_quota_bytes: 25_000_000_000,
    },
    users: [
      { id: "u_4f9c2a1e", email: "alice@example.dev", name: "Alice Chen", is_admin: true, sessions: 7, devices: 4, created: "2026-01-04" },
      { id: "u_91ab33", email: "bren@example.dev", name: "Bren Owusu", is_admin: false, sessions: 23, devices: 2, created: "2026-01-12" },
      { id: "u_2c8f10", email: "kai@example.dev", name: "Kai Nakamura", is_admin: false, sessions: 4, devices: 1, created: "2026-02-08" },
      { id: "u_a17b22", email: "dev-bot@example.dev", name: "Dev Bot", is_admin: false, sessions: 102, devices: 1, created: "2026-02-20" },
      { id: "u_55e8aa", email: "rosa@example.dev", name: "Rosa García", is_admin: true, sessions: 18, devices: 3, created: "2026-03-01" },
      { id: "u_77fa01", email: "wei@example.dev", name: "Wei Zhang", is_admin: false, sessions: 11, devices: 2, created: "2026-03-14" },
    ],
    server_mcps: [
      { name: "minimax", tools: 4, resources: 2, prompts: 1, enabled: true },
      { name: "linear", tools: 7, resources: 0, prompts: 0, enabled: true },
      { name: "github-public", tools: 6, resources: 0, prompts: 1, enabled: false },
    ],
    metrics: { active_users: 4, sessions_today: 41, tool_calls_24h: 1248, llm_tokens_24h: 4_120_400 },
  },
};

const fmtBytes = (n) => {
  if (n < 1024) return n + " B";
  if (n < 1024 ** 2) return (n / 1024).toFixed(1) + " KB";
  if (n < 1024 ** 3) return (n / 1024 ** 2).toFixed(1) + " MB";
  return (n / 1024 ** 3).toFixed(2) + " GB";
};
const fmtNum = (n) => n.toLocaleString();

window.PLEXUS_DATA = PLEXUS_DATA;
window.fmtBytes = fmtBytes;
window.fmtNum = fmtNum;
