// Channels — Discord + Telegram bot configs.

const ChannelCard = ({ name, icon, cfg, accent }) => (
  <div className="panel">
    <div className="panel-head">
      <span style={{ color: accent }}>{icon}</span>
      <div className="panel-title" style={{ textTransform: "capitalize" }}>{name}</div>
      {cfg?.configured ? <Chip kind="ok">configured</Chip> : <Chip>not configured</Chip>}
    </div>
    <div className="panel-body">
      {cfg?.configured ? (
        <>
          <div style={{ display: "grid", gridTemplateColumns: "140px 1fr", gap: "10px 16px", fontSize: 12 }}>
            <div className="muted">Bot token</div>
            <div className="mono id-mono" style={{ color: "var(--fg-0)" }}>{cfg.bot_token_masked} <button className="btn btn-sm btn-icon btn-ghost" style={{ marginLeft: 6 }}><Icons.Copy /></button></div>
            <div className="muted">Partner chat_id</div>
            <div className="mono id-mono">{cfg.partner_chat_id} <span className="muted">(messages from this id are not wrapped)</span></div>
            <div className="muted">Allow-list</div>
            <div>{cfg.allow_list_count} additional id{cfg.allow_list_count === 1 ? "" : "s"}</div>
          </div>
          <hr className="sep" />
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn btn-sm">Edit</button>
            <button className="btn btn-sm">Manage allow-list</button>
            <div style={{ flex: 1 }} />
            <button className="btn btn-sm btn-danger">Disconnect</button>
          </div>
        </>
      ) : (
        <Empty title={`No ${name} configured`} sub="Set a bot token and partner chat_id to start receiving messages." action={<button className="btn btn-sm btn-primary">Configure</button>} />
      )}
    </div>
  </div>
);

const ChannelsView = () => {
  const c = window.PLEXUS_DATA.channels;
  return (
    <div className="view-pad">
      <SectionHeader
        title="Channels"
        sub="Discord and Telegram integrations. Messages from your declared partner_chat_id reach the agent unwrapped; everyone else lands as `[untrusted message from <name>]:` content (ADR-007, ADR-074)."
      />
      <div className="grid grid-2">
        <ChannelCard name="discord" cfg={c.discord} icon={<Icons.Discord />} accent="oklch(70% 0.13 270)" />
        <ChannelCard name="telegram" cfg={c.telegram} icon={<Icons.Telegram />} accent="oklch(70% 0.14 230)" />
      </div>
    </div>
  );
};

window.ChannelsView = ChannelsView;
