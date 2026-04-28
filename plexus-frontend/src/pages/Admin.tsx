import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Eye, EyeOff, Plus, Trash2 } from 'lucide-react'
import { api, ApiError } from '../lib/api'
import type { LlmConfig, RateLimit, DefaultSoul, McpServerEntry, AdminUser } from '../lib/types'

type Tab = 'llm' | 'soul' | 'rate' | 'mcp' | 'users'

export default function Admin() {
  const [tab, setTab] = useState<Tab>('llm')
  const navigate = useNavigate()

  const tabs: { id: Tab; label: string }[] = [
    { id: 'llm', label: 'LLM' },
    { id: 'soul', label: 'Default Soul' },
    { id: 'rate', label: 'Rate Limit' },
    { id: 'mcp', label: 'Server MCPs' },
    { id: 'users', label: 'Users' },
  ]

  return (
    <div className="min-h-screen" style={{ background: 'var(--bg)', color: 'var(--text)' }}>
      <div className="max-w-3xl mx-auto px-6 py-8">
        <div className="flex items-center gap-3 mb-8">
          <button
            onClick={() => navigate('/chat')}
            className="p-1 rounded hover:bg-[#1a2332] transition-colors"
            style={{ color: 'var(--muted)' }}
          >
            <ArrowLeft size={18} />
          </button>
          <h1 className="text-lg font-semibold" style={{ color: 'var(--accent)' }}>Admin</h1>
        </div>

        <div className="flex gap-1 border-b mb-8" style={{ borderColor: 'var(--border)' }}>
          {tabs.map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className="px-4 py-2 text-sm -mb-px transition-colors"
              style={{
                color: tab === t.id ? 'var(--accent)' : 'var(--muted)',
                borderBottom: tab === t.id ? '2px solid var(--accent)' : '2px solid transparent',
              }}
            >
              {t.label}
            </button>
          ))}
        </div>

        {tab === 'llm' && <LlmTab />}
        {tab === 'soul' && <DefaultSoulTab />}
        {tab === 'rate' && <RateLimitTab />}
        {tab === 'mcp' && <ServerMcpTab />}
        {tab === 'users' && <UsersTab />}
      </div>
    </div>
  )
}

// ── LLM Tab ───────────────────────────────────────────────────────────────────

function LlmTab() {
  const [form, setForm] = useState<LlmConfig>({
    api_base: '',
    model: '',
    api_key: '',
    context_window: 128000,
  })
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<LlmConfig>('/api/llm-config').then(c => setForm(c)).catch(() => {})
  }, [])

  async function save() {
    setLoading(true)
    try {
      await api.put('/api/llm-config', form)
      setMsg('LLM config saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <AdminField
        label="API Base URL"
        value={form.api_base}
        onChange={v => setForm(f => ({ ...f, api_base: v }))}
        placeholder="https://api.openai.com/v1"
      />
      <AdminField
        label="Model"
        value={form.model}
        onChange={v => setForm(f => ({ ...f, model: v }))}
        placeholder="gpt-4o"
      />
      <AdminField
        label="API Key"
        value={form.api_key}
        onChange={v => setForm(f => ({ ...f, api_key: v }))}
        type="password"
        placeholder="sk-..."
      />
      <AdminField
        label="Context Window (tokens)"
        value={String(form.context_window)}
        onChange={v => setForm(f => ({ ...f, context_window: parseInt(v) || 128000 }))}
        type="number"
      />
      <AdminSave onClick={save} loading={loading} />
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Default Soul Tab ──────────────────────────────────────────────────────────

function DefaultSoulTab() {
  const [soul, setSoul] = useState('')
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<DefaultSoul>('/api/admin/default-soul').then(r => setSoul(r.soul ?? '')).catch(() => {})
  }, [])

  async function save() {
    setLoading(true)
    try {
      await api.put('/api/admin/default-soul', { soul })
      setMsg('Default soul saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-xs" style={{ color: 'var(--muted)' }}>
        Applied to all users who haven't set their own soul.
      </p>
      <textarea
        value={soul}
        onChange={e => setSoul(e.target.value)}
        rows={10}
        placeholder="You are a helpful AI assistant."
        className="w-full rounded-lg p-3 text-sm font-mono resize-y outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
      <AdminSave onClick={save} loading={loading} />
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Rate Limit Tab ────────────────────────────────────────────────────────────

function RateLimitTab() {
  const [rateLimit, setRateLimit] = useState(0)
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<RateLimit>('/api/admin/rate-limit').then(r => setRateLimit(r.rate_limit_per_min)).catch(() => {})
  }, [])

  async function save() {
    setLoading(true)
    try {
      await api.put('/api/admin/rate-limit', { rate_limit_per_min: rateLimit })
      setMsg('Rate limit saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-xs" style={{ color: 'var(--muted)' }}>
        Messages per minute per user. 0 = unlimited.
      </p>
      <div className="flex items-center gap-4">
        <input
          type="number"
          min={0}
          value={rateLimit}
          onChange={e => setRateLimit(parseInt(e.target.value) || 0)}
          className="w-32 rounded-lg px-3 py-2 text-sm outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <span className="text-xs" style={{ color: 'var(--muted)' }}>messages / minute</span>
      </div>
      <AdminSave onClick={save} loading={loading} />
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Server MCPs Tab ───────────────────────────────────────────────────────────
//
// Structured editor for server-wide MCP configs (FR5 / spec §10.3). List rows
// show each installed MCP with masked env values; Add / Remove / Edit flow
// through a modal. Saves replace-all via PUT /api/server-mcp.

type Transport = 'stdio' | 'http'

function transportOf(m: McpServerEntry): Transport {
  if (m.transport_type === 'http') return 'http'
  if (m.transport_type === 'stdio') return 'stdio'
  // Infer when legacy entries omit transport_type.
  return m.url ? 'http' : 'stdio'
}

function emptyEntry(): McpServerEntry {
  return {
    name: '',
    transport_type: 'stdio',
    command: '',
    args: [],
    env: {},
    url: null,
    headers: null,
    tool_timeout: null,
    enabled: true,
  }
}

function ServerMcpTab() {
  const [servers, setServers] = useState<McpServerEntry[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // Modal state: `null` = closed, `{ index: null }` = add, `{ index: n }` = edit row n.
  const [editing, setEditing] = useState<{ index: number | null } | null>(null)
  const [confirmRemove, setConfirmRemove] = useState<number | null>(null)

  async function load() {
    try {
      const r = await api.get<{ mcp_servers: McpServerEntry[] }>('/api/server-mcp')
      setServers(r.mcp_servers ?? [])
      setLoadError(null)
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Failed to load MCP config')
    }
  }

  useEffect(() => { void load() }, [])

  async function persist(next: McpServerEntry[]): Promise<boolean> {
    setSaving(true)
    setErr(null)
    setMsg(null)
    try {
      await api.put('/api/server-mcp', { mcp_servers: next })
      // Confirm server state by refetching (spec §10.3).
      await load()
      setMsg('Server MCP saved. Servers will restart.')
      return true
    } catch (e) {
      if (e instanceof ApiError) {
        setErr(e.message)
      } else {
        setErr(e instanceof Error ? e.message : 'Save failed')
      }
      return false
    } finally {
      setSaving(false)
    }
  }

  async function onSaveEntry(entry: McpServerEntry, index: number | null): Promise<boolean> {
    if (!servers) return false
    const next = [...servers]
    if (index === null) next.push(entry)
    else next[index] = entry
    return persist(next)
  }

  async function onRemove(index: number) {
    if (!servers) return
    const next = servers.filter((_, i) => i !== index)
    await persist(next)
    setConfirmRemove(null)
  }

  if (loadError) {
    return (
      <div className="text-xs" style={{ color: '#ef4444' }}>{loadError}</div>
    )
  }
  if (!servers) {
    return (
      <div className="text-xs" style={{ color: 'var(--muted)' }}>Loading…</div>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <p className="text-xs" style={{ color: 'var(--muted)' }}>
          MCP servers available to all users on this instance.
        </p>
        <button
          onClick={() => setEditing({ index: null })}
          className="flex items-center gap-1 text-xs px-3 py-1.5 rounded font-semibold"
          style={{ background: 'var(--accent)', color: '#000' }}
        >
          <Plus size={14} /> Add MCP
        </button>
      </div>

      {servers.length === 0 && (
        <p className="text-xs" style={{ color: 'var(--muted)' }}>
          No MCP servers configured.
        </p>
      )}

      <ul className="list-none p-0 m-0 flex flex-col gap-2">
        {servers.map((s, i) => (
          <McpRow
            key={`${s.name}-${i}`}
            entry={s}
            onEdit={() => setEditing({ index: i })}
            onRemove={() => setConfirmRemove(i)}
          />
        ))}
      </ul>

      {err && <p className="text-xs" style={{ color: '#ef4444' }}>{err}</p>}
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}

      {editing && (
        <McpModal
          initial={editing.index === null ? emptyEntry() : servers[editing.index]}
          existingNames={servers
            .filter((_, i) => i !== editing.index)
            .map(s => s.name)}
          saving={saving}
          onCancel={() => setEditing(null)}
          onSave={async (entry) => {
            const ok = await onSaveEntry(entry, editing.index)
            if (ok) setEditing(null)
          }}
        />
      )}

      {confirmRemove !== null && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center"
          style={{ background: 'rgba(0,0,0,0.5)' }}
          onClick={() => !saving && setConfirmRemove(null)}
        >
          <div
            className="rounded p-6 max-w-md w-full"
            style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
            onClick={(e) => e.stopPropagation()}
          >
            <h2 className="text-lg font-semibold mb-2" style={{ color: '#ef4444' }}>
              Remove {servers[confirmRemove].name}?
            </h2>
            <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
              The MCP server will be removed from this instance and restarted
              without it. Users will lose access to its tools.
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirmRemove(null)}
                disabled={saving}
                className="text-sm px-3 py-1 rounded"
                style={{ border: '1px solid var(--border)' }}
              >
                Cancel
              </button>
              <button
                onClick={() => void onRemove(confirmRemove)}
                disabled={saving}
                className="text-sm px-3 py-1 rounded"
                style={{ background: '#ef4444', color: 'white' }}
              >
                {saving ? 'Removing…' : 'Remove'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function McpRow({
  entry,
  onEdit,
  onRemove,
}: {
  entry: McpServerEntry
  onEdit: () => void
  onRemove: () => void
}) {
  const transport = transportOf(entry)
  const envEntries = Object.entries(entry.env ?? {})

  return (
    <li
      className="rounded border px-4 py-3 flex flex-col gap-2"
      style={{ borderColor: 'var(--border)', background: 'var(--card)' }}
    >
      <div className="flex items-center gap-2">
        <div className="flex-1 min-w-0">
          <div className="text-sm font-semibold truncate">
            {entry.name || <span style={{ color: 'var(--muted)' }}>(unnamed)</span>}
            <span
              className="ml-2 text-[10px] uppercase tracking-wider px-1.5 py-0.5 rounded"
              style={{ background: 'var(--bg)', color: 'var(--muted)', border: '1px solid var(--border)' }}
            >
              {transport}
            </span>
            {!entry.enabled && (
              <span
                className="ml-2 text-[10px] uppercase tracking-wider"
                style={{ color: 'var(--muted)' }}
              >
                disabled
              </span>
            )}
          </div>
          <div
            className="text-xs font-mono truncate"
            style={{ color: 'var(--muted)' }}
          >
            {transport === 'http'
              ? (entry.url ?? '(no url)')
              : [entry.command, ...(entry.args ?? [])].filter(Boolean).join(' ') || '(no command)'}
          </div>
        </div>
        <button
          onClick={onEdit}
          className="text-xs px-2 py-1 rounded"
          style={{ border: '1px solid var(--border)' }}
        >
          Edit
        </button>
        <button
          onClick={onRemove}
          className="text-xs px-2 py-1 rounded flex items-center gap-1"
          style={{ border: '1px solid var(--border)', color: '#ef4444' }}
        >
          <Trash2 size={12} /> Remove
        </button>
      </div>

      {envEntries.length > 0 && (
        <div className="flex flex-col gap-1 pt-1 border-t" style={{ borderColor: 'var(--border)' }}>
          <div className="text-[10px] uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
            env
          </div>
          <ul className="list-none p-0 m-0 flex flex-col gap-0.5">
            {envEntries.map(([k, v]) => (
              <EnvRow key={k} name={k} value={v} />
            ))}
          </ul>
        </div>
      )}
    </li>
  )
}

function EnvRow({ name, value }: { name: string; value: string }) {
  const [shown, setShown] = useState(false)
  return (
    <li className="flex items-center gap-2 text-xs font-mono">
      <span style={{ color: 'var(--accent)' }}>{name}</span>
      <span style={{ color: 'var(--muted)' }}>=</span>
      <span className="flex-1 truncate" style={{ color: 'var(--text)' }}>
        {shown ? value : '••••••••'}
      </span>
      <button
        type="button"
        onClick={() => setShown(s => !s)}
        className="p-1 rounded"
        style={{ color: 'var(--muted)' }}
        title={shown ? 'Hide' : 'Show'}
      >
        {shown ? <EyeOff size={12} /> : <Eye size={12} />}
      </button>
    </li>
  )
}

function McpModal({
  initial,
  existingNames,
  saving,
  onCancel,
  onSave,
}: {
  initial: McpServerEntry
  existingNames: string[]
  saving: boolean
  onCancel: () => void
  onSave: (entry: McpServerEntry) => void | Promise<void>
}) {
  const [draft, setDraft] = useState<McpServerEntry>(() => ({
    ...initial,
    transport_type: initial.transport_type ?? transportOf(initial),
    args: initial.args ?? [],
    env: initial.env ?? {},
  }))
  const [localErr, setLocalErr] = useState<string | null>(null)

  // Stdio: keep args as a single space-separated string in the form.
  const [argsText, setArgsText] = useState<string>((initial.args ?? []).join(' '))

  // Env list — sorted by insertion order via array of tuples to allow blank keys.
  const [envPairs, setEnvPairs] = useState<Array<{ key: string; value: string; shown: boolean }>>(
    () => Object.entries(initial.env ?? {}).map(([k, v]) => ({ key: k, value: v, shown: false })),
  )

  const transport: Transport = draft.transport_type === 'http' ? 'http' : 'stdio'

  function handleSubmit() {
    setLocalErr(null)
    const name = draft.name.trim()
    if (!name) {
      setLocalErr('Name is required.')
      return
    }
    if (existingNames.includes(name)) {
      setLocalErr('Name must be unique.')
      return
    }

    let entry: McpServerEntry
    if (transport === 'http') {
      const url = (draft.url ?? '').trim()
      if (!url) {
        setLocalErr('URL is required for http transport.')
        return
      }
      entry = {
        name,
        transport_type: 'http',
        command: '',
        args: [],
        env: null,
        url,
        headers: draft.headers ?? null,
        tool_timeout: draft.tool_timeout ?? null,
        enabled: draft.enabled,
      }
    } else {
      const command = draft.command.trim()
      if (!command) {
        setLocalErr('Command is required for stdio transport.')
        return
      }
      const args = argsText.trim() === '' ? [] : argsText.trim().split(/\s+/)
      const envObj: Record<string, string> = {}
      for (const { key, value } of envPairs) {
        const k = key.trim()
        if (!k) continue
        envObj[k] = value
      }
      entry = {
        name,
        transport_type: 'stdio',
        command,
        args,
        env: Object.keys(envObj).length > 0 ? envObj : null,
        url: null,
        headers: null,
        tool_timeout: draft.tool_timeout ?? null,
        enabled: draft.enabled,
      }
    }

    void onSave(entry)
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={() => !saving && onCancel()}
    >
      <div
        className="rounded p-6 max-w-lg w-full max-h-[90vh] overflow-y-auto flex flex-col gap-4"
        style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold">
          {initial.name ? 'Edit MCP Server' : 'Add MCP Server'}
        </h2>

        <AdminField
          label="Name"
          value={draft.name}
          onChange={v => setDraft(d => ({ ...d, name: v }))}
          placeholder="my-mcp"
        />

        <div className="flex flex-col gap-1">
          <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
            Transport
          </label>
          <select
            value={transport}
            onChange={e => setDraft(d => ({ ...d, transport_type: e.target.value as Transport }))}
            className="rounded-lg px-3 py-2 text-sm outline-none border"
            style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
          >
            <option value="stdio">stdio (command)</option>
            <option value="http">http (url)</option>
          </select>
        </div>

        {transport === 'http' ? (
          <AdminField
            label="URL"
            value={draft.url ?? ''}
            onChange={v => setDraft(d => ({ ...d, url: v }))}
            placeholder="https://example.com/mcp"
          />
        ) : (
          <>
            <AdminField
              label="Command"
              value={draft.command}
              onChange={v => setDraft(d => ({ ...d, command: v }))}
              placeholder="uvx"
            />
            <AdminField
              label="Args (space-separated)"
              value={argsText}
              onChange={setArgsText}
              placeholder="minimax-mcp --flag value"
            />

            {/* env editor */}
            <div className="flex flex-col gap-1">
              <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
                Env
              </label>
              <div className="flex flex-col gap-1">
                {envPairs.map((p, i) => (
                  <div key={i} className="flex items-center gap-1">
                    <input
                      type="text"
                      value={p.key}
                      onChange={e => setEnvPairs(prev => prev.map((x, j) => j === i ? { ...x, key: e.target.value } : x))}
                      placeholder="KEY"
                      className="w-32 rounded-lg px-2 py-1.5 text-xs font-mono outline-none border"
                      style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
                    />
                    <span style={{ color: 'var(--muted)' }}>=</span>
                    <input
                      type={p.shown ? 'text' : 'password'}
                      value={p.value}
                      onChange={e => setEnvPairs(prev => prev.map((x, j) => j === i ? { ...x, value: e.target.value } : x))}
                      placeholder="value"
                      className="flex-1 rounded-lg px-2 py-1.5 text-xs font-mono outline-none border"
                      style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
                    />
                    <button
                      type="button"
                      onClick={() => setEnvPairs(prev => prev.map((x, j) => j === i ? { ...x, shown: !x.shown } : x))}
                      className="p-1 rounded"
                      style={{ color: 'var(--muted)' }}
                      title={p.shown ? 'Hide' : 'Show'}
                    >
                      {p.shown ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                    <button
                      type="button"
                      onClick={() => setEnvPairs(prev => prev.filter((_, j) => j !== i))}
                      className="p-1 rounded"
                      style={{ color: '#ef4444' }}
                      title="Remove"
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                ))}
                <button
                  type="button"
                  onClick={() => setEnvPairs(prev => [...prev, { key: '', value: '', shown: false }])}
                  className="self-start text-xs px-2 py-1 rounded flex items-center gap-1"
                  style={{ border: '1px solid var(--border)', color: 'var(--muted)' }}
                >
                  <Plus size={12} /> Add env var
                </button>
              </div>
            </div>
          </>
        )}

        <div className="flex items-center gap-2">
          <input
            id="mcp-enabled"
            type="checkbox"
            checked={draft.enabled}
            onChange={e => setDraft(d => ({ ...d, enabled: e.target.checked }))}
          />
          <label htmlFor="mcp-enabled" className="text-xs" style={{ color: 'var(--muted)' }}>
            Enabled
          </label>
        </div>

        {localErr && <p className="text-xs" style={{ color: '#ef4444' }}>{localErr}</p>}

        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            disabled={saving}
            className="text-sm px-3 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={saving}
            className="text-sm px-3 py-1 rounded font-semibold"
            style={{ background: 'var(--accent)', color: '#000' }}
          >
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Users Tab ─────────────────────────────────────────────────────────────────

function UsersTab() {
  const [users, setUsers] = useState<AdminUser[]>([])
  const [filter, setFilter] = useState('')
  const [msg, setMsg] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<AdminUser | null>(null)
  const [deleting, setDeleting] = useState(false)

  useEffect(() => {
    void load()
  }, [])

  async function load() {
    try {
      const data = await api.get<AdminUser[]>('/api/admin/users')
      setUsers(data)
      setMsg(null)
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'load failed')
    }
  }

  async function doDelete(u: AdminUser) {
    setDeleting(true)
    try {
      await api.delete<{ message: string }>(`/api/admin/users/${encodeURIComponent(u.user_id)}`)
      setConfirmDelete(null)
      await load()
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'delete failed')
    } finally {
      setDeleting(false)
    }
  }

  const visible = users.filter(
    (u) =>
      filter === '' ||
      u.email.toLowerCase().includes(filter.toLowerCase()) ||
      (u.display_name ?? '').toLowerCase().includes(filter.toLowerCase()) ||
      u.user_id.toLowerCase().includes(filter.toLowerCase()),
  )

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Search by email, display name, or user_id…"
          className="flex-1 px-3 py-2 rounded text-sm outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <span className="text-xs" style={{ color: 'var(--muted)' }}>
          {visible.length} / {users.length}
        </span>
      </div>
      {msg && (
        <div className="text-sm" style={{ color: '#ef4444' }}>
          {msg}
        </div>
      )}
      <ul className="list-none p-0 m-0">
        {visible.map((u) => (
          <li
            key={u.user_id}
            className="flex items-center gap-2 py-2 border-b"
            style={{ borderColor: 'var(--border)' }}
          >
            <div className="flex-1 min-w-0">
              <div className="text-sm">
                <strong>{u.display_name ?? u.email}</strong>
                {u.is_admin && (
                  <span className="ml-2 text-xs" style={{ color: 'var(--accent)' }}>
                    admin
                  </span>
                )}
              </div>
              <div className="text-xs" style={{ color: 'var(--muted)' }}>
                {u.email} · {u.user_id} · joined {new Date(u.created_at).toLocaleDateString()}
              </div>
            </div>
            <button
              onClick={() => setConfirmDelete(u)}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)', color: '#ef4444' }}
            >
              Delete
            </button>
          </li>
        ))}
      </ul>
      {confirmDelete && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center"
          style={{ background: 'rgba(0,0,0,0.5)' }}
          onClick={() => !deleting && setConfirmDelete(null)}
        >
          <div
            className="rounded p-6 max-w-md w-full"
            style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
            onClick={(e) => e.stopPropagation()}
          >
            <h2 className="text-lg font-semibold mb-2" style={{ color: '#ef4444' }}>
              Delete {confirmDelete.display_name ?? confirmDelete.email}?
            </h2>
            <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
              This will permanently delete the user and all their data: sessions, messages,
              channels, files, skills. <strong>This cannot be undone.</strong>
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirmDelete(null)}
                disabled={deleting}
                className="text-sm px-3 py-1 rounded"
                style={{ border: '1px solid var(--border)' }}
              >
                Cancel
              </button>
              <button
                onClick={() => void doDelete(confirmDelete)}
                disabled={deleting}
                className="text-sm px-3 py-1 rounded"
                style={{ background: '#ef4444', color: 'white' }}
              >
                {deleting ? 'Deleting…' : 'Delete Forever'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

// ── Shared primitives ─────────────────────────────────────────────────────────

function AdminField({
  label,
  value,
  onChange,
  type = 'text',
  placeholder,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>{label}</label>
      <input
        type={type}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        className="rounded-lg px-3 py-2 text-sm outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
    </div>
  )
}

function AdminSave({ onClick, loading }: { onClick: () => void; loading: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={loading}
      className="self-start px-4 py-1.5 rounded-lg text-xs font-semibold uppercase tracking-wider transition-all disabled:opacity-50"
      style={{ background: 'var(--accent)', color: '#000', boxShadow: loading ? 'none' : '0 0 8px var(--accent)' }}
    >
      {loading ? 'Saving…' : 'Save'}
    </button>
  )
}
