// Chat — the main agent-loop surface.

const ToolHint = ({ tool, args, status }) => {
  return (
    <div style={{
      display: "inline-flex", alignItems: "center", gap: 8,
      padding: "4px 10px", margin: "4px 0",
      background: "var(--bg-1)",
      border: "1px solid var(--line)",
      borderRadius: 999,
      fontSize: 11,
      color: "var(--fg-2)",
      fontFamily: "var(--font-mono)",
    }}>
      {status === "running" ? <Dot kind="pulse" /> : <Icons.Tool />}
      <span style={{ color: "var(--fg-1)" }}>{tool}</span>
      <span style={{ color: "var(--fg-3)" }}>·</span>
      <span style={{
        color: "var(--accent-fg)",
        background: "var(--accent-soft)",
        padding: "0 6px",
        borderRadius: 3,
      }}>{args.device || "server"}</span>
    </div>
  );
};

const ToolBlock = ({ tool, args, result }) => {
  const [open, setOpen] = React.useState(false);
  const argSummary = React.useMemo(() => {
    const entries = Object.entries(args).slice(0, 2);
    return entries.map(([k, v]) =>
      typeof v === "string" && v.length > 32 ? `${k}="${v.slice(0, 32)}…"` : `${k}=${JSON.stringify(v)}`
    ).join(" ");
  }, [args]);

  return (
    <div style={{
      border: "1px solid var(--line)",
      borderRadius: "var(--radius)",
      background: "var(--bg-1)",
      overflow: "hidden",
      marginTop: 8,
    }}>
      <div
        onClick={() => setOpen(!open)}
        style={{
          padding: "8px 12px",
          display: "flex", alignItems: "center", gap: 10,
          cursor: "pointer",
          borderBottom: open ? "1px solid var(--line)" : "none",
        }}
      >
        <span style={{
          color: result?.ok ? "var(--ok)" : "var(--danger)",
          flexShrink: 0,
        }}>
          {result?.ok ? <Icons.Check /> : <Icons.X />}
        </span>
        <span className="mono" style={{ fontSize: 12, color: "var(--fg-0)", fontWeight: 500 }}>{tool.name}</span>
        <span style={{
          fontSize: 10, padding: "1px 6px",
          background: "var(--accent-soft)", color: "var(--accent-fg)",
          border: "1px solid var(--accent-line)",
          borderRadius: 3, fontFamily: "var(--font-mono)",
        }}>plexus_device={tool.device}</span>
        <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)", flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{argSummary}</span>
        {result ? (
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>
            {result.bytes ? fmtBytes(result.bytes) : null}{result.replacements ? `${result.replacements} repl.` : null}
          </span>
        ) : null}
        <span style={{ color: "var(--fg-3)", transform: open ? "rotate(90deg)" : "rotate(0deg)", transition: "transform 0.15s" }}>
          <Icons.ChevronRight />
        </span>
      </div>
      {open ? (
        <div style={{ padding: "10px 12px", borderTop: "1px solid var(--line)", background: "var(--bg-0)" }}>
          <div style={{ fontSize: 10, color: "var(--fg-3)", textTransform: "uppercase", letterSpacing: "0.06em", fontFamily: "var(--font-mono)", marginBottom: 6 }}>Args</div>
          <pre style={{
            margin: 0, fontFamily: "var(--font-mono)", fontSize: 11,
            color: "var(--fg-1)", whiteSpace: "pre-wrap", wordBreak: "break-word",
            background: "var(--bg-1)", padding: "8px 10px", borderRadius: "var(--radius-sm)",
            border: "1px solid var(--line)",
          }}>{JSON.stringify(args, null, 2)}</pre>

          {result ? (
            <>
              <div style={{ fontSize: 10, color: "var(--fg-3)", textTransform: "uppercase", letterSpacing: "0.06em", fontFamily: "var(--font-mono)", marginTop: 12, marginBottom: 6 }}>Result {result.ok ? "" : "(error)"}</div>
              <pre style={{
                margin: 0, fontFamily: "var(--font-mono)", fontSize: 11,
                color: result.ok ? "var(--fg-1)" : "var(--danger)",
                whiteSpace: "pre-wrap", wordBreak: "break-word",
                background: "var(--bg-1)", padding: "8px 10px", borderRadius: "var(--radius-sm)",
                border: "1px solid var(--line)",
                maxHeight: 200, overflow: "auto",
              }}>{result.stdout}</pre>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
};

const formatInline = (text) => {
  // Simple **bold** and `code` rendering
  const parts = [];
  let i = 0; let buf = "";
  let key = 0;
  const flush = () => { if (buf) { parts.push(buf); buf = ""; } };
  while (i < text.length) {
    if (text.startsWith("**", i)) {
      const end = text.indexOf("**", i + 2);
      if (end !== -1) { flush(); parts.push(<strong key={key++} style={{ color: "var(--fg-0)", fontWeight: 600 }}>{text.slice(i + 2, end)}</strong>); i = end + 2; continue; }
    }
    if (text[i] === "`") {
      const end = text.indexOf("`", i + 1);
      if (end !== -1) { flush(); parts.push(<code key={key++} style={{ fontFamily: "var(--font-mono)", fontSize: "0.9em", background: "var(--bg-2)", padding: "1px 5px", borderRadius: 3, color: "var(--fg-0)" }}>{text.slice(i + 1, end)}</code>); i = end + 1; continue; }
    }
    buf += text[i]; i++;
  }
  flush();
  return parts;
};

const MessageBlock = ({ block }) => {
  if (block.kind === "text") {
    return <div style={{ marginBottom: 8 }}>{formatInline(block.text)}</div>;
  }
  if (block.kind === "list") {
    return (
      <ol style={{ margin: "8px 0", paddingLeft: 22, color: "var(--fg-1)" }}>
        {block.items.map((it, i) => <li key={i} style={{ marginBottom: 6 }}>{formatInline(it)}</li>)}
      </ol>
    );
  }
  if (block.kind === "code") {
    return <CodeBlock lang={block.lang}>{block.text}</CodeBlock>;
  }
  if (block.kind === "buttons") {
    return (
      <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 10 }}>
        {block.buttons[0].map((b, i) => (
          <button key={i} className="btn btn-sm" style={{ borderColor: "var(--accent-line)", color: "var(--accent-fg)" }}>{b}</button>
        ))}
      </div>
    );
  }
  return null;
};

const Avatar = ({ role, name }) => {
  if (role === "user") {
    return (
      <div className="user-avatar" style={{ width: 26, height: 26, fontSize: 11 }}>{name?.split(" ").map(p => p[0]).slice(0, 2).join("") || "U"}</div>
    );
  }
  return (
    <div style={{
      width: 26, height: 26, borderRadius: 6,
      background: "var(--bg-2)",
      border: "1px solid var(--line)",
      display: "grid", placeItems: "center",
      color: "var(--accent-fg)",
      flexShrink: 0,
    }}>
      <Icons.Sparkle />
    </div>
  );
};

const Message = ({ msg, showHints, showDeviceBadges }) => {
  if (msg.role === "tool") {
    return <ToolBlock tool={msg.tool} args={msg.args} result={msg.result} />;
  }

  return (
    <div style={{ display: "flex", gap: 12, padding: "16px 0" }}>
      <Avatar role={msg.role} name={msg.role === "user" ? PLEXUS_DATA.user.name : "Plexus"} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "baseline", gap: 8, marginBottom: 6 }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--fg-0)" }}>
            {msg.role === "user" ? PLEXUS_DATA.user.name : "Plexus"}
          </span>
          {msg.role === "assistant" && msg.is_compaction_summary ? (
            <Chip kind="warn">compaction summary</Chip>
          ) : null}
          <span style={{ fontSize: 11, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }} className="id-mono">{msg.t}</span>
        </div>
        <div style={{ color: "var(--fg-1)", fontSize: 13, lineHeight: 1.55 }}>
          {msg.text ? formatInline(msg.text) : null}
          {msg.blocks?.map((b, i) => <MessageBlock key={i} block={b} />)}
        </div>
      </div>
    </div>
  );
};

const Composer = ({ streaming, onSend, onStop }) => {
  const [v, setV] = React.useState("");
  const [device, setDevice] = React.useState("server");
  const taRef = React.useRef(null);

  React.useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = Math.min(180, ta.scrollHeight) + "px";
  }, [v]);

  const submit = () => {
    if (!v.trim()) return;
    onSend(v);
    setV("");
  };

  const onKey = e => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div style={{
      borderTop: "1px solid var(--line)",
      padding: 16,
      background: "var(--bg-0)",
    }}>
      <div style={{
        background: "var(--bg-1)",
        border: "1px solid var(--line)",
        borderRadius: "var(--radius-lg)",
        padding: 8,
        transition: "border-color 0.15s, box-shadow 0.15s",
      }}>
        <textarea
          ref={taRef}
          value={v}
          onChange={e => setV(e.target.value)}
          onKeyDown={onKey}
          placeholder="Message Plexus.  Shift+Enter for newline.  /attach to upload."
          rows={1}
          style={{
            width: "100%",
            background: "transparent",
            border: "none",
            outline: "none",
            color: "var(--fg-0)",
            fontFamily: "var(--font-ui)",
            fontSize: 13,
            resize: "none",
            padding: "8px 6px",
            lineHeight: 1.5,
          }}
        />
        <div style={{ display: "flex", alignItems: "center", gap: 8, paddingTop: 8, borderTop: "1px solid var(--line)" }}>
          <button className="btn btn-sm btn-ghost" title="Attach"><Icons.Image /></button>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 11, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }} className="id-mono">
            ⌘K commands
          </span>
          {streaming ? (
            <button className="btn btn-sm btn-danger" onClick={onStop}>
              <Icons.Stop /> Stop
            </button>
          ) : (
            <button className="btn btn-sm btn-primary" onClick={submit}>
              <Icons.Send /> Send
            </button>
          )}
        </div>
      </div>
      <div style={{
        marginTop: 8, display: "flex", gap: 8, alignItems: "center",
        fontSize: 11, color: "var(--fg-3)", fontFamily: "var(--font-mono)",
      }} className="id-mono">
        <span>session_key=web:alice:main</span>
        <span>·</span>
        <span>14 msgs · ~3.2k tokens · context 12% used</span>
      </div>
    </div>
  );
};

const StreamingIndicator = () => (
  <div style={{
    display: "flex", alignItems: "center", gap: 8,
    padding: "10px 0",
    fontSize: 12, color: "var(--fg-3)",
    fontFamily: "var(--font-mono)",
  }} className="id-mono">
    <Dot kind="pulse" />
    <span>plexus is thinking · iter 4/200</span>
    <span style={{ color: "var(--fg-4)" }}>·</span>
    <span>tool_call dispatched: <span style={{ color: "var(--accent-fg)" }}>web_fetch → server</span></span>
  </div>
);

const ChatView = ({ tweaks }) => {
  const [activeId, setActiveId] = React.useState("s_8a4d");
  const session = PLEXUS_DATA.sessions.find(s => s.id === activeId);
  const messages = activeId === "s_8a4d" ? PLEXUS_DATA.messages : [];

  const [streaming, setStreaming] = React.useState(true);
  const scrollRef = React.useRef(null);

  React.useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [activeId, messages.length]);

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
      <SessionsList active={activeId} onPick={setActiveId} />

      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, background: "var(--bg-0)" }}>
        {/* Session header */}
        <div style={{
          padding: "10px 20px",
          borderBottom: "1px solid var(--line)",
          display: "flex", alignItems: "center", gap: 12,
          height: 48, flexShrink: 0,
        }}>
          <ChannelGlyph channel={session?.channel || "web"} size={16} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: "var(--fg-0)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
              {session?.title || "—"}
            </div>
            <div className="mono id-mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>
              {session?.session_key}
            </div>
          </div>
          {streaming ? <Chip kind="accent"><Dot kind="pulse" /> live</Chip> : null}
          <button className="btn btn-sm btn-ghost btn-icon" title="Session settings"><Icons.Settings /></button>
        </div>

        {/* Message stream */}
        <div ref={scrollRef} style={{ flex: 1, overflow: "auto", padding: "8px 20px" }}>
          {messages.length === 0 ? (
            <div style={{ paddingTop: 80 }}>
              <Empty title="No messages yet" sub="This session has no history yet. Send a message to start." />
            </div>
          ) : null}

          {messages.map((m, i) => {
            // Insert tool_call hints between messages, if enabled
            const prev = messages[i - 1];
            const showHint = tweaks.showInlineHints && m.role === "tool" && prev?.role === "assistant";
            return (
              <React.Fragment key={m.id}>
                {showHint ? (
                  <div style={{ paddingLeft: 38 }}>
                    <ToolHint tool={m.tool.name} args={{ device: m.tool.device }} status="running" />
                  </div>
                ) : null}
                <div style={{ paddingLeft: m.role === "tool" ? 38 : 0 }}>
                  <Message msg={m} showHints={tweaks.showInlineHints} showDeviceBadges={tweaks.showDeviceBadges} />
                </div>
              </React.Fragment>
            );
          })}

          {streaming ? (
            <div style={{ paddingLeft: 38 }}>
              <StreamingIndicator />
            </div>
          ) : null}

          <div style={{ height: 16 }} />
        </div>

        <Composer
          streaming={streaming}
          onSend={() => setStreaming(true)}
          onStop={() => setStreaming(false)}
        />
      </div>
    </div>
  );
};

window.ChatView = ChatView;
