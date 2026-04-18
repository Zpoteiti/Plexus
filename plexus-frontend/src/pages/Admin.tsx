import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { api } from '../lib/api'
import type { LlmConfig, RateLimit, DefaultSoul, McpServerEntry, AdminUser } from '../lib/types'

type Tab = 'llm' | 'soul' | 'rate' | 'mcp' | 'users'

export default function Admin() {
  const [tab, setTab] = useState<Tab>('llm')
  const navigate = useNavigate()

  const tabs: { id: Tab; label: string }[] = [
    { id: 'llm', label: 'LLM' },
    { id: 'soul', label: 'Default Soul' },
    { id: 'rate', label: 'Rate Limit' },
    { id: 'mcp', label: 'Server MCP' },
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

// ── Server MCP Tab ────────────────────────────────────────────────────────────

function ServerMcpTab() {
  const [json, setJson] = useState('[]')
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api
      .get<{ mcp_servers: McpServerEntry[] }>('/api/server-mcp')
      .then(r => setJson(JSON.stringify(r.mcp_servers, null, 2)))
      .catch(() => {})
  }, [])

  async function save() {
    setLoading(true)
    try {
      const parsed = JSON.parse(json) as McpServerEntry[]
      await api.put('/api/server-mcp', { mcp_servers: parsed })
      setMsg('Server MCP saved. Servers will restart.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Invalid JSON')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-xs" style={{ color: 'var(--muted)' }}>
        MCP servers available to all users on this instance.
      </p>
      <textarea
        value={json}
        onChange={e => setJson(e.target.value)}
        rows={16}
        placeholder={'[{"name":"minimax","command":"uvx","args":["minimax-mcp"],"env":{"MINIMAX_API_KEY":"..."},"enabled":true}]'}
        className="w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
      <AdminSave onClick={save} loading={loading} />
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
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
