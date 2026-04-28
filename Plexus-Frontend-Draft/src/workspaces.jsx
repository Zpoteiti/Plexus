// Workspaces view — personal + shared

const WorkspaceCard = ({ w }) => {
  const pct = (w.bytes_used / w.quota_bytes) * 100;
  return (
    <div className="panel" style={{ marginBottom: 12 }}>
      <div style={{ padding: "16px", display: "flex", gap: 16, alignItems: "center" }}>
        <div style={{
          width: 36, height: 36, borderRadius: "var(--radius)",
          background: w.kind === "shared" ? "var(--accent-soft)" : "var(--bg-2)",
          color: w.kind === "shared" ? "var(--accent-fg)" : "var(--fg-2)",
          display: "grid", placeItems: "center", flexShrink: 0,
          border: "1px solid var(--line)",
        }}>
          {w.kind === "shared" ? <Icons.Sphere /> : <Icons.Folder />}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: "var(--fg-0)" }} className="mono id-mono">{w.name}</span>
            <Chip kind={w.kind === "shared" ? "accent" : "default"}>{w.kind}</Chip>
            {w.locked ? <Chip kind="warn">soft-locked</Chip> : null}
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 6 }}>
            <div style={{ flex: 1, maxWidth: 320 }}>
              <Progress value={w.bytes_used} max={w.quota_bytes} />
            </div>
            <span className="mono id-mono" style={{ fontSize: 11, color: "var(--fg-2)" }}>
              {fmtBytes(w.bytes_used)} / {fmtBytes(w.quota_bytes)} ({pct.toFixed(0)}%)
            </span>
          </div>
          <div className="mono id-mono" style={{ fontSize: 11, color: "var(--fg-3)", marginTop: 6 }}>
            /{w.path_segment}/ · {fmtNum(w.file_count)} files · {w.members} member{w.members === 1 ? "" : "s"}
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <button className="btn btn-sm">Browse</button>
          <button className="btn btn-sm btn-icon"><Icons.More /></button>
        </div>
      </div>
    </div>
  );
};

const FileBrowser = () => {
  const entries = [
    { name: "skills/", kind: "dir", size: null, mtime: "2026-04-22" },
    { name: "projects/", kind: "dir", size: null, mtime: "2026-04-26" },
    { name: ".attachments/", kind: "dir", size: null, mtime: "today" },
    { name: "memory.md", kind: "file", size: 12_400, mtime: "today 14:08" },
    { name: "soul.md", kind: "file", size: 3_180, mtime: "Apr 12" },
    { name: "nanobot-port-notes.md", kind: "file", size: 84_900, mtime: "yesterday" },
    { name: "schema.sql", kind: "file", size: 5_240, mtime: "Apr 23" },
  ];

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">personal · alice</div>
        <div className="panel-sub mono id-mono">/u_4f9c2a1e/</div>
        <div style={{ flex: 1 }} />
        <button className="btn btn-sm"><Icons.Plus /> Upload</button>
        <button className="btn btn-sm btn-icon"><Icons.Search /></button>
      </div>
      <table className="tbl">
        <thead>
          <tr><th style={{ width: 40 }}></th><th>Name</th><th>Size</th><th>Modified</th><th></th></tr>
        </thead>
        <tbody>
          {entries.map(e => (
            <tr key={e.name}>
              <td>
                <span style={{ color: e.kind === "dir" ? "var(--accent-fg)" : "var(--fg-3)" }}>
                  {e.kind === "dir" ? <Icons.Folder /> : <Icons.File />}
                </span>
              </td>
              <td className="mono id-mono" style={{ color: "var(--fg-0)" }}>{e.name}</td>
              <td className="mono id-mono" style={{ color: "var(--fg-3)" }}>{e.size ? fmtBytes(e.size) : "—"}</td>
              <td className="mono id-mono" style={{ color: "var(--fg-3)" }}>{e.mtime}</td>
              <td style={{ textAlign: "right" }}><button className="btn btn-sm btn-icon btn-ghost"><Icons.More /></button></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

const WorkspacesView = () => {
  const workspaces = window.PLEXUS_DATA.workspaces;
  const total_used = workspaces.reduce((a, w) => a + w.bytes_used, 0);
  const total_quota = workspaces.reduce((a, w) => a + w.quota_bytes, 0);
  const total_files = workspaces.reduce((a, w) => a + w.file_count, 0);

  return (
    <div className="view-pad">
      <SectionHeader
        title="Workspaces"
        sub="Personal workspace + any shared spaces you've been invited to. Workspace_fs handles quota, SKILL.md validation, and symlink boundaries."
        actions={
          <>
            <button className="btn btn-sm">Quota report</button>
            <button className="btn btn-sm btn-primary"><Icons.Plus /> New shared workspace</button>
          </>
        }
      />

      <div className="grid grid-3" style={{ marginBottom: 20 }}>
        <Stat label="Total usage" value={fmtBytes(total_used)} trend={`across ${workspaces.length} workspaces`} />
        <Stat label="Files" value={fmtNum(total_files)} trend="indexed by workspace_fs" />
        <Stat label="Quota headroom" value={fmtBytes(total_quota - total_used)} trend={`${((1 - total_used / total_quota) * 100).toFixed(0)}% remaining`} />
      </div>

      <div style={{ marginBottom: 20 }}>
        {workspaces.map(w => <WorkspaceCard key={w.name} w={w} />)}
      </div>

      <div style={{ fontSize: 11, color: "var(--fg-3)", textTransform: "uppercase", fontFamily: "var(--font-mono)", letterSpacing: "0.06em", marginBottom: 10 }}>
        Personal workspace files
      </div>
      <FileBrowser />
    </div>
  );
};

window.WorkspacesView = WorkspacesView;
