// Cron view — scheduled agent invocations.

const CronRow = ({ j }) => {
  const sched = j.one_shot ? "one-shot" :
    j.schedule.startsWith("every") ? j.schedule :
    j.schedule.includes("*") ? "cron" : "scheduled";

  return (
    <tr>
      <td><Chip kind={j.one_shot ? "warn" : "default"}>{j.one_shot ? "one-shot" : "recurring"}</Chip></td>
      <td className="mono id-mono" style={{ color: "var(--fg-0)", fontWeight: 500 }}>{j.name}</td>
      <td className="mono id-mono">{j.schedule}{j.tz ? ` ${j.tz}` : ""}</td>
      <td style={{ maxWidth: 320, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{j.description}</td>
      <td><span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><ChannelGlyph channel={j.channel} size={12} /> <span className="mono id-mono">{j.channel}</span></span></td>
      <td className="mono id-mono" style={{ color: "var(--fg-3)" }}>{j.last_fired_at || "—"}</td>
      <td className="mono id-mono" style={{ color: "var(--fg-1)" }}>{j.next_fire_at}</td>
      <td>{j.deliver ? <Chip kind="ok">deliver</Chip> : <Chip>silent</Chip>}</td>
      <td style={{ textAlign: "right" }}>
        <button className="btn btn-sm btn-icon btn-ghost"><Icons.Trash /></button>
      </td>
    </tr>
  );
};

const CronView = () => {
  const jobs = window.PLEXUS_DATA.cron_jobs;
  return (
    <div className="view-pad">
      <SectionHeader
        title="Cron"
        sub="Scheduled agent invocations. Each firing creates or continues a dedicated cron session and replies to the channel + chat_id where the cron was set up (ADR-053)."
        actions={<button className="btn btn-sm btn-primary"><Icons.Plus /> New cron</button>}
      />

      <div className="grid grid-3" style={{ marginBottom: 20 }}>
        <Stat label="Active jobs" value={jobs.filter(j => !j.one_shot).length} trend="recurring" />
        <Stat label="One-shots queued" value={jobs.filter(j => j.one_shot).length} trend="self-destruct after fire" />
        <Stat label="Next fire" value="33m" trend="drink-water → telegram" />
      </div>

      <div className="panel">
        <table className="tbl">
          <thead>
            <tr>
              <th>Type</th>
              <th>Name</th>
              <th>Schedule</th>
              <th>Description</th>
              <th>Channel</th>
              <th>Last fired</th>
              <th>Next fire</th>
              <th>Mode</th>
              <th></th>
            </tr>
          </thead>
          <tbody>{jobs.map(j => <CronRow key={j.id} j={j} />)}</tbody>
        </table>
      </div>
    </div>
  );
};

window.CronView = CronView;
