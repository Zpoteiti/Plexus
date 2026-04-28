// Devices view — register/configure clients, fs_policy, MCPs, tokens.

const DeviceCard = ({ d, expanded, onToggle, onEdit }) => {
  return (
    <div className="panel" style={{ marginBottom: 12 }}>
      <div
        onClick={onToggle}
        style={{
          padding: "14px 16px",
          display: "flex", alignItems: "center", gap: 14,
          cursor: "pointer",
        }}
      >
        <Dot kind={d.online ? "ok" : "off"} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: "var(--fg-0)" }} className="mono id-mono">{d.name}</span>
            <Chip kind="default">{d.os}</Chip>
            <Chip kind={d.fs_policy === "unrestricted" ? "warn" : "default"}>fs:{d.fs_policy}</Chip>
            <Chip kind="default">bwrap:{d.caps.sandbox}</Chip>
            {d.tools_in_flight > 0 ? <Chip kind="accent"><Dot kind="pulse" /> {d.tools_in_flight} in flight</Chip> : null}
          </div>
          <div className="mono id-mono" style={{ fontSize: 11, color: "var(--fg-3)", marginTop: 4 }}>
            {d.workspace_path} · v{d.client_version} · last seen {d.last_seen}
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }} onClick={e => e.stopPropagation()}>
          <button className="btn btn-sm" onClick={onEdit}>Configure</button>
          <button className="btn btn-sm btn-icon"><Icons.More /></button>
        </div>
      </div>

      {expanded ? (
        <div style={{ padding: "0 16px 16px", borderTop: "1px solid var(--line)" }}>
          <div className="grid grid-2" style={{ marginTop: 14 }}>
            <div>
              <div style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", marginBottom: 8 }}>SSRF whitelist</div>
              {d.ssrf_whitelist.length === 0 ? (
                <div style={{ fontSize: 12, color: "var(--fg-3)" }} className="id-mono">— none —</div>
              ) : (
                <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                  {d.ssrf_whitelist.map(h => <Chip key={h}>{h}</Chip>)}
                </div>
              )}
            </div>
            <div>
              <div style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", marginBottom: 8 }}>Capabilities</div>
              <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                <Chip kind={d.caps.exec ? "ok" : "default"}>exec:{String(d.caps.exec)}</Chip>
                <Chip>fs:{d.caps.fs}</Chip>
                <Chip>shell_max:{d.shell_timeout_max}s</Chip>
              </div>
            </div>
          </div>

          <div style={{ marginTop: 16 }}>
            <div style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", marginBottom: 8 }}>
              MCP servers ({d.mcp_servers.length})
            </div>
            {d.mcp_servers.length === 0 ? (
              <div style={{ fontSize: 12, color: "var(--fg-3)" }}>None registered.</div>
            ) : (
              <table className="tbl" style={{ background: "var(--bg-0)", borderRadius: "var(--radius)", border: "1px solid var(--line)", overflow: "hidden" }}>
                <thead>
                  <tr><th>Server</th><th>Tools</th><th>Resources</th><th>Prompts</th><th></th></tr>
                </thead>
                <tbody>
                  {d.mcp_servers.map(m => (
                    <tr key={m.name}>
                      <td className="mono id-mono" style={{ color: "var(--fg-0)" }}>{m.name}</td>
                      <td>{m.tools}</td>
                      <td>{m.resources}</td>
                      <td>{m.prompts}</td>
                      <td style={{ textAlign: "right" }}><Chip kind="ok">enabled</Chip></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
};

const DevicesView = () => {
  const devices = window.PLEXUS_DATA.devices;
  const [expanded, setExpanded] = React.useState("alice-laptop");
  const [showNew, setShowNew] = React.useState(false);

  const online = devices.filter(d => d.online).length;
  const total_mcps = devices.reduce((acc, d) => acc + d.mcp_servers.reduce((a, m) => a + m.tools + m.resources + m.prompts, 0), 0);
  const in_flight = devices.reduce((acc, d) => acc + d.tools_in_flight, 0);

  return (
    <div className="view-pad">
      <SectionHeader
        title="Devices"
        sub="Execution nodes connected to your account. Each one runs the client binary and exposes tools over a WebSocket back to the server."
        actions={
          <>
            <button className="btn btn-sm"><Icons.Refresh /> Reload</button>
            <button className="btn btn-sm btn-primary" onClick={() => setShowNew(true)}><Icons.Plus /> Register device</button>
          </>
        }
      />

      <div className="grid grid-3" style={{ marginBottom: 20 }}>
        <Stat label="Online" value={online} unit={`/ ${devices.length}`} trend="all heartbeats nominal" />
        <Stat label="Tools in flight" value={in_flight} trend="across all connected devices" />
        <Stat label="MCP capabilities" value={total_mcps} trend="tools + resources + prompts" />
      </div>

      {showNew ? (
        <div className="panel" style={{ marginBottom: 16, borderColor: "var(--accent-line)" }}>
          <div className="panel-head" style={{ background: "var(--accent-soft)" }}>
            <div className="panel-title">Register new device</div>
            <div className="panel-sub">The token is shown once — copy it before closing.</div>
            <div style={{ flex: 1 }} />
            <button className="btn btn-sm btn-icon btn-ghost" onClick={() => setShowNew(false)}><Icons.X /></button>
          </div>
          <div className="panel-body">
            <div className="grid grid-2">
              <div>
                <label style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", display: "block", marginBottom: 6 }}>Friendly name</label>
                <input className="input input-mono" placeholder="e.g. cloud-vm-2" style={{ width: "100%" }} />
              </div>
              <div>
                <label style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", display: "block", marginBottom: 6 }}>fs_policy</label>
                <select className="input" style={{ width: "100%" }}>
                  <option>sandbox (recommended)</option>
                  <option>unrestricted (requires confirmation)</option>
                </select>
              </div>
            </div>

            <div style={{ marginTop: 16, padding: 14, background: "var(--bg-0)", border: "1px solid var(--line)", borderRadius: "var(--radius)" }}>
              <div style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", marginBottom: 8 }}>Token (will be shown once)</div>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <code style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--fg-0)", padding: "6px 10px", background: "var(--bg-1)", borderRadius: 4, border: "1px solid var(--line)" }}>
                  plx_3a8c•••••••••••••••••••••••••••••••••
                </code>
                <button className="btn btn-sm"><Icons.Copy /> Copy</button>
              </div>
              <div style={{ fontSize: 11, color: "var(--fg-3)", marginTop: 8, fontFamily: "var(--font-mono)" }} className="id-mono">
                $ export PLEXUS_DEVICE_TOKEN=plx_3a8c…<br/>
                $ export PLEXUS_SERVER_WS_URL=ws://localhost:8080/ws<br/>
                $ cd plexus-client && cargo run
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {devices.map(d => (
        <DeviceCard
          key={d.name}
          d={d}
          expanded={expanded === d.name}
          onToggle={() => setExpanded(expanded === d.name ? null : d.name)}
        />
      ))}
    </div>
  );
};

window.DevicesView = DevicesView;
