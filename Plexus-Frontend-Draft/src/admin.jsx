// Admin console — system_config, users, server-side MCPs.

const ConfigRow = ({ k, v, mono = true, sensitive = false }) => (
  <tr>
    <td className="mono id-mono" style={{ color: "var(--fg-2)", width: 320 }}>{k}</td>
    <td className={mono ? "mono id-mono" : ""} style={{ color: "var(--fg-0)" }}>
      {sensitive ? "••••••••••••••••••" : v}
    </td>
    <td style={{ textAlign: "right", width: 80 }}>
      <button className="btn btn-sm btn-ghost btn-icon"><Icons.Settings /></button>
    </td>
  </tr>
);

const AdminView = () => {
  const a = window.PLEXUS_DATA.admin;
  return (
    <div className="view-pad">
      <SectionHeader
        title="Admin"
        sub="System-wide config, user management, server-side MCPs. Anyone registering with ADMIN_TOKEN gets the keys to this page."
        actions={<Chip kind="warn">admin only</Chip>}
      />

      <div className="grid grid-3" style={{ marginBottom: 20 }}>
        <Stat label="Active users (24h)" value={a.metrics.active_users} unit={`/ ${a.users.length}`} trend="logged a session" />
        <Stat label="Sessions today" value={a.metrics.sessions_today} trend="across all channels" />
        <Stat label="LLM tokens (24h)" value={(a.metrics.llm_tokens_24h / 1000).toFixed(0)} unit="k" trend={`${fmtNum(a.metrics.tool_calls_24h)} tool calls`} />
      </div>

      <div className="panel" style={{ marginBottom: 20 }}>
        <div className="panel-head">
          <div className="panel-title">system_config</div>
          <div className="panel-sub">PostgreSQL key/value store. Edits push live without restart.</div>
        </div>
        <table className="tbl">
          <tbody>
            <ConfigRow k="llm_endpoint" v={a.config.llm_endpoint} />
            <ConfigRow k="llm_api_key" v="" sensitive />
            <ConfigRow k="llm_model" v={a.config.llm_model} />
            <ConfigRow k="llm_max_context_tokens" v={fmtNum(a.config.llm_max_context_tokens)} />
            <ConfigRow k="llm_compaction_threshold_tokens" v={fmtNum(a.config.llm_compaction_threshold_tokens)} />
            <ConfigRow k="quota_bytes" v={fmtBytes(a.config.quota_bytes) + " / user"} />
            <ConfigRow k="shared_workspace_quota_bytes" v={fmtBytes(a.config.shared_workspace_quota_bytes) + " ceiling"} />
          </tbody>
        </table>
      </div>

      <div className="panel" style={{ marginBottom: 20 }}>
        <div className="panel-head">
          <div className="panel-title">Users <span style={{ color: "var(--fg-3)", fontWeight: 400 }}>· {a.users.length}</span></div>
          <div style={{ flex: 1 }} />
          <input className="input" placeholder="Search users…" style={{ width: 200 }} />
        </div>
        <table className="tbl">
          <thead>
            <tr>
              <th>Name</th><th>Email</th><th>User ID</th><th>Sessions</th><th>Devices</th><th>Joined</th><th></th>
            </tr>
          </thead>
          <tbody>
            {a.users.map(u => (
              <tr key={u.id}>
                <td style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <div className="user-avatar" style={{ width: 22, height: 22, fontSize: 10 }}>{u.name.split(" ").map(p => p[0]).join("")}</div>
                  <span style={{ color: "var(--fg-0)" }}>{u.name}</span>
                  {u.is_admin ? <Chip kind="accent">admin</Chip> : null}
                </td>
                <td className="mono id-mono">{u.email}</td>
                <td className="mono id-mono" style={{ color: "var(--fg-3)" }}>{u.id}</td>
                <td>{u.sessions}</td>
                <td>{u.devices}</td>
                <td className="mono id-mono" style={{ color: "var(--fg-3)" }}>{u.created}</td>
                <td style={{ textAlign: "right" }}><button className="btn btn-sm btn-icon btn-ghost"><Icons.More /></button></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">Server-side MCP servers</div>
          <div className="panel-sub">Admin-managed; available to every user's agent loop.</div>
          <div style={{ flex: 1 }} />
          <button className="btn btn-sm btn-primary"><Icons.Plus /> Add MCP</button>
        </div>
        <table className="tbl">
          <thead><tr><th>Server</th><th>Tools</th><th>Resources</th><th>Prompts</th><th>Status</th><th></th></tr></thead>
          <tbody>
            {a.server_mcps.map(m => (
              <tr key={m.name}>
                <td className="mono id-mono" style={{ color: "var(--fg-0)" }}>{m.name}</td>
                <td>{m.tools}</td>
                <td>{m.resources}</td>
                <td>{m.prompts}</td>
                <td>{m.enabled ? <Chip kind="ok">enabled</Chip> : <Chip>disabled</Chip>}</td>
                <td style={{ textAlign: "right" }}><button className="btn btn-sm btn-icon btn-ghost"><Icons.More /></button></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
};

window.AdminView = AdminView;
