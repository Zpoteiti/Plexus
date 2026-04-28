// Shared UI helpers.

const Dot = ({ kind = "off" }) => <span className={`dot dot-${kind}`} />;

const Chip = ({ kind = "default", children, mono = true, ...rest }) => (
  <span className={`chip ${kind === "default" ? "" : "chip-" + kind}`} style={mono ? null : { fontFamily: "var(--font-ui)" }} {...rest}>{children}</span>
);

const Stat = ({ label, value, unit, trend }) => (
  <div className="stat">
    <div className="stat-label">{label}</div>
    <div className="stat-value">
      {value}{unit ? <span className="unit">{unit}</span> : null}
    </div>
    {trend ? <div className="stat-trend">{trend}</div> : null}
  </div>
);

const Progress = ({ value, max, kind }) => {
  const pct = Math.max(0, Math.min(100, (value / max) * 100));
  let cls = "";
  if (pct > 95) cls = "danger";
  else if (pct > 80) cls = "warn";
  if (kind) cls = kind;
  return (
    <div className="progress">
      <div className={`progress-bar ${cls}`} style={{ width: pct + "%" }} />
    </div>
  );
};

const ChannelGlyph = ({ channel, size = 14 }) => {
  const size16 = `${size}px`;
  const map = {
    web: { icon: <Icons.Web />, color: "var(--accent-fg)" },
    discord: { icon: <Icons.Discord />, color: "oklch(70% 0.13 270)" },
    telegram: { icon: <Icons.Telegram />, color: "oklch(70% 0.14 230)" },
    cron: { icon: <Icons.Cron />, color: "oklch(74% 0.16 70)" },
  };
  const m = map[channel] || map.web;
  return (
    <span style={{
      width: size16, height: size16,
      display: "inline-grid", placeItems: "center",
      color: m.color,
      flexShrink: 0,
    }}>
      {React.cloneElement(m.icon, { })}
    </span>
  );
};

// Section headers
const SectionHeader = ({ title, sub, actions }) => (
  <div className="section-h">
    <div>
      <h2>{title}</h2>
      {sub ? <div className="sub">{sub}</div> : null}
    </div>
    <div style={{ display: "flex", gap: 8 }}>{actions}</div>
  </div>
);

// Empty state
const Empty = ({ title, sub, action }) => (
  <div style={{
    padding: "48px 24px",
    textAlign: "center",
    color: "var(--fg-3)",
    border: "1px dashed var(--line)",
    borderRadius: "var(--radius-lg)",
    background: "var(--bg-1)",
  }}>
    <div style={{ fontSize: 13, color: "var(--fg-1)", fontWeight: 500, marginBottom: 4 }}>{title}</div>
    {sub ? <div style={{ fontSize: 12, marginBottom: action ? 16 : 0 }}>{sub}</div> : null}
    {action}
  </div>
);

// Code block
const CodeBlock = ({ children, lang }) => (
  <div style={{
    background: "var(--bg-0)",
    border: "1px solid var(--line)",
    borderRadius: "var(--radius)",
    fontFamily: "var(--font-mono)",
    fontSize: 12,
    overflow: "hidden",
    marginTop: 4,
  }}>
    {lang ? (
      <div style={{
        padding: "4px 10px",
        fontSize: 10,
        color: "var(--fg-3)",
        textTransform: "uppercase",
        letterSpacing: "0.06em",
        borderBottom: "1px solid var(--line)",
      }}>{lang}</div>
    ) : null}
    <pre style={{ margin: 0, padding: "10px 12px", overflow: "auto", color: "var(--fg-1)" }}>{children}</pre>
  </div>
);

window.Dot = Dot;
window.Chip = Chip;
window.Stat = Stat;
window.Progress = Progress;
window.ChannelGlyph = ChannelGlyph;
window.SectionHeader = SectionHeader;
window.Empty = Empty;
window.CodeBlock = CodeBlock;
