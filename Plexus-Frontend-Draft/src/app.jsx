// App shell — sidebar nav, topbar, view router, tweaks integration.

const NAV = [
  { id: "chat", label: "Chat", icon: <Icons.Chat />, badge: "3" },
  { id: "devices", label: "Devices", icon: <Icons.Device />, dot: true },
  { id: "workspaces", label: "Workspaces", icon: <Icons.Workspace /> },
  { id: "cron", label: "Cron", icon: <Icons.Cron /> },
  { id: "channels", label: "Channels", icon: <Icons.Channel /> },
];

const NAV_ADMIN = [
  { id: "admin", label: "Admin", icon: <Icons.Admin /> },
];

const Sidebar = ({ active, onPick, mode }) => {
  const isRail = mode === "rail";
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">P</div>
        {!isRail ? (
          <div style={{ minWidth: 0 }}>
            <div className="brand-text">Plexus</div>
            <div className="brand-sub">v0.3.0 · alice</div>
          </div>
        ) : null}
      </div>

      <div className="nav-section">
        <div className="nav-label">Surfaces</div>
        {NAV.map(n => (
          <div key={n.id} className={"nav-item" + (active === n.id ? " active" : "")} onClick={() => onPick(n.id)} title={n.label}>
            {n.icon}
            <span className="nav-text">{n.label}</span>
            {n.badge ? <span className="nav-badge">{n.badge}</span> : null}
            {n.dot ? <span className="nav-badge dot" /> : null}
          </div>
        ))}
      </div>

      <div className="nav-section">
        <div className="nav-label">System</div>
        {NAV_ADMIN.map(n => (
          <div key={n.id} className={"nav-item" + (active === n.id ? " active" : "")} onClick={() => onPick(n.id)} title={n.label}>
            {n.icon}
            <span className="nav-text">{n.label}</span>
          </div>
        ))}
      </div>

      <div className="sidebar-bottom">
        <div className="user-card" title={PLEXUS_DATA.user.email}>
          <div className="user-avatar">{PLEXUS_DATA.user.initials}</div>
          <div className="user-meta">
            <div className="user-name">{PLEXUS_DATA.user.name}</div>
            <div className="user-email">{PLEXUS_DATA.user.email}</div>
          </div>
        </div>
      </div>
    </aside>
  );
};

const Topbar = ({ active, onToggleTweaks }) => {
  const labelMap = {
    chat: "Chat",
    devices: "Devices",
    workspaces: "Workspaces",
    cron: "Cron",
    channels: "Channels",
    admin: "Admin",
  };
  return (
    <div className="topbar">
      <div className="topbar-title">
        <span className="crumb-root">plexus</span>
        <span className="crumb-sep">/</span>
        <span className="crumb-leaf">{labelMap[active]}</span>
      </div>
      <div className="spacer" />
      <span className="kbd">⌘K</span>
      <button className="btn btn-sm btn-ghost btn-icon" title="Notifications"><Icons.Bell /></button>
      <button className="btn btn-sm btn-ghost btn-icon" title="Tweaks" onClick={onToggleTweaks}><Icons.Sliders /></button>
    </div>
  );
};

const App = () => {
  const [tweaks, setTweak] = window.useTweaks(window.TWEAK_DEFAULTS);
  const [active, setActive] = React.useState("chat");
  const [tweaksOpen, setTweaksOpen] = React.useState(false);

  // Apply theme/accent/density/font etc. to the document
  React.useEffect(() => {
    const r = document.documentElement;
    r.dataset.theme = tweaks.theme;
    r.dataset.accent = tweaks.accent;
    r.dataset.density = tweaks.density;
    r.dataset.fontpair = tweaks.fontPair;
    r.dataset.monoids = String(tweaks.monoForIds);
  }, [tweaks]);

  const sidebarMode = tweaks.sidebar === "rail-with-labels" ? "default"
                    : tweaks.sidebar === "rail-only" ? "rail"
                    : "hidden";

  const view = active === "chat" ? <ChatView tweaks={tweaks} />
            : active === "devices" ? <div className="view"><DevicesView /></div>
            : active === "workspaces" ? <div className="view"><WorkspacesView /></div>
            : active === "cron" ? <div className="view"><CronView /></div>
            : active === "channels" ? <div className="view"><ChannelsView /></div>
            : active === "admin" ? <div className="view"><AdminView /></div>
            : null;

  return (
    <div className="shell" data-sidebar={sidebarMode}>
      <Sidebar active={active} onPick={setActive} mode={sidebarMode === "rail" ? "rail" : "default"} />
      <div className="main">
        <Topbar active={active} onToggleTweaks={() => setTweaksOpen(!tweaksOpen)} />
        {active === "chat" ? view : <div className="view">{view}</div>}
      </div>

      {tweaksOpen ? (
        <TweaksPanel onClose={() => setTweaksOpen(false)}>
          <TweakSection label="Appearance" />
          <TweakRadio label="Theme" value={tweaks.theme} onChange={v => setTweak("theme", v)}
            options={["dark", "light"]} />
          <TweakSelect label="Accent" value={tweaks.accent} onChange={v => setTweak("accent", v)}
            options={["indigo", "cyan", "amber", "green", "magenta", "white"]} />
          <TweakSelect label="Font pair" value={tweaks.fontPair} onChange={v => setTweak("fontPair", v)}
            options={[
              { value: "inter-jetbrains", label: "Inter + JetBrains" },
              { value: "geist", label: "Geist + Geist Mono" },
              { value: "ibm-plex", label: "IBM Plex" },
            ]} />

          <TweakSection label="Density & layout" />
          <TweakRadio label="Density" value={tweaks.density} onChange={v => setTweak("density", v)}
            options={["compact", "comfortable", "roomy"]} />
          <TweakSelect label="Sidebar" value={tweaks.sidebar} onChange={v => setTweak("sidebar", v)}
            options={[
              { value: "rail-with-labels", label: "Full (icons + labels)" },
              { value: "rail-only", label: "Rail (icons only)" },
              { value: "hidden", label: "Hidden" },
            ]} />

          <TweakSection label="Chat surface" />
          <TweakToggle label="Inline tool-call hints" value={tweaks.showInlineHints} onChange={v => setTweak("showInlineHints", v)} />
          <TweakToggle label="Device badges" value={tweaks.showDeviceBadges} onChange={v => setTweak("showDeviceBadges", v)} />
          <TweakToggle label="Monospace IDs" value={tweaks.monoForIds} onChange={v => setTweak("monoForIds", v)} />
        </TweaksPanel>
      ) : null}
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
