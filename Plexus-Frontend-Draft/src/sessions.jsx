// Sessions sidebar (between left nav and chat pane)
const SessionsList = ({ active, onPick }) => {
  const sessions = window.PLEXUS_DATA.sessions;
  const [filter, setFilter] = React.useState("all");
  const [q, setQ] = React.useState("");

  const filtered = sessions.filter(s => {
    if (filter !== "all" && s.channel !== filter) return false;
    if (q && !s.title.toLowerCase().includes(q.toLowerCase())) return false;
    return true;
  });

  const filters = [
    { id: "all", label: "All" },
    { id: "web", label: "Web" },
    { id: "discord", label: "Discord" },
    { id: "telegram", label: "Telegram" },
    { id: "cron", label: "Cron" },
  ];

  return (
    <div style={{
      width: 296, flexShrink: 0,
      borderRight: "1px solid var(--line)",
      display: "flex", flexDirection: "column",
      background: "var(--bg-1)",
    }}>
      <div style={{
        padding: "10px 12px",
        borderBottom: "1px solid var(--line)",
        display: "flex", alignItems: "center", gap: 8,
        height: 48, flexShrink: 0,
      }}>
        <div style={{ fontWeight: 600, fontSize: 13, color: "var(--fg-0)", flex: 1 }}>Sessions</div>
        <button className="btn btn-sm btn-icon" title="New session"><Icons.Plus /></button>
      </div>

      <div style={{ padding: "10px 12px", borderBottom: "1px solid var(--line)" }}>
        <div style={{ position: "relative" }}>
          <span style={{ position: "absolute", left: 8, top: "50%", transform: "translateY(-50%)", color: "var(--fg-3)" }}><Icons.Search /></span>
          <input
            className="input"
            value={q}
            onChange={e => setQ(e.target.value)}
            placeholder="Search sessions"
            style={{ width: "100%", paddingLeft: 28 }}
          />
        </div>
        <div style={{ display: "flex", gap: 4, marginTop: 8, flexWrap: "wrap" }}>
          {filters.map(f => (
            <button
              key={f.id}
              onClick={() => setFilter(f.id)}
              className={"btn btn-sm" + (filter === f.id ? " btn-primary" : " btn-ghost")}
              style={{ fontSize: 11, height: 22, padding: "0 8px" }}
            >{f.label}</button>
          ))}
        </div>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "4px 6px" }}>
        {filtered.map(s => (
          <div
            key={s.id}
            onClick={() => onPick(s.id)}
            style={{
              padding: "10px 12px",
              borderRadius: "var(--radius)",
              cursor: "pointer",
              background: s.id === active ? "var(--accent-soft)" : "transparent",
              border: s.id === active ? "1px solid var(--accent-line)" : "1px solid transparent",
              marginBottom: 2,
            }}
            onMouseEnter={e => { if (s.id !== active) e.currentTarget.style.background = "var(--bg-hover)"; }}
            onMouseLeave={e => { if (s.id !== active) e.currentTarget.style.background = "transparent"; }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
              <ChannelGlyph channel={s.channel} />
              <span className="mono id-mono" style={{ fontSize: 10, color: "var(--fg-3)", textTransform: "uppercase", letterSpacing: "0.06em" }}>{s.channel}</span>
              {s.streaming ? <Dot kind="pulse" /> : null}
              {s.unread ? <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--accent)", marginLeft: "auto" }} /> : null}
              <span style={{ fontSize: 11, color: "var(--fg-3)", fontFamily: "var(--font-mono)", marginLeft: s.unread ? 6 : "auto" }}>{s.last_at}</span>
            </div>
            <div style={{ fontSize: 13, color: "var(--fg-0)", fontWeight: s.unread ? 600 : 500, lineHeight: 1.35 }}>{s.title}</div>
            <div style={{ fontSize: 10, color: "var(--fg-4)", marginTop: 4, fontFamily: "var(--font-mono)" }} className="id-mono">
              {s.session_key} · {s.msg_count} msgs
            </div>
          </div>
        ))}
        {filtered.length === 0 ? (
          <div style={{ padding: 24, textAlign: "center", color: "var(--fg-3)", fontSize: 12 }}>No matching sessions.</div>
        ) : null}
      </div>
    </div>
  );
};

window.SessionsList = SessionsList;
