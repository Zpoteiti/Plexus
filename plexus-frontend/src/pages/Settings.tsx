import { useState, useEffect, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { api, ApiError } from '../lib/api'
import { useAuthStore } from '../store/auth'
import type {
  User,
  Device,
  DeviceToken,
  DeviceConfig,
  McpServerEntry,
  DiscordConfig,
  TelegramConfig,
  CronJob,
  WorkspaceSkill,
} from '../lib/types'

type Tab = 'profile' | 'devices' | 'channels' | 'skills' | 'cron'

export default function Settings() {
  const [tab, setTab] = useState<Tab>('profile')
  const navigate = useNavigate()

  const tabs: { id: Tab; label: string }[] = [
    { id: 'profile', label: 'Profile' },
    { id: 'devices', label: 'Devices' },
    { id: 'channels', label: 'Channels' },
    { id: 'skills', label: 'Skills' },
    { id: 'cron', label: 'Cron' },
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
          <h1 className="text-lg font-semibold" style={{ color: 'var(--accent)' }}>Settings</h1>
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

        {tab === 'profile' && <ProfileTab />}
        {tab === 'devices' && <DevicesTab />}
        {tab === 'channels' && <ChannelsTab />}
        {tab === 'skills' && <SkillsTab />}
        {tab === 'cron' && <CronTab />}
      </div>
    </div>
  )
}

// ── Profile Tab ───────────────────────────────────────────────────────────────

function ProfileTab() {
  const [profile, setProfile] = useState<User | null>(null)
  const [displayName, setDisplayName] = useState('')
  const [saving, setSaving] = useState(false)
  const [msg, setMsg] = useState('')
  const logout = useAuthStore(s => s.logout)
  const refreshProfile = useAuthStore(s => s.refreshProfile)
  const navigate = useNavigate()

  useEffect(() => {
    void (async () => {
      const p = await api.get<User>('/api/user/profile')
      setProfile(p)
      setDisplayName(p.display_name ?? '')
    })()
  }, [])

  const [deleteOpen, setDeleteOpen] = useState(false)
  const [deletePassword, setDeletePassword] = useState('')
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)

  async function saveDisplayName() {
    setSaving(true)
    try {
      await api.patch('/api/user/display-name', { display_name: displayName })
      await refreshProfile()
      setMsg('Name saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSaving(false)
    }
  }

  async function confirmDelete() {
    setDeleting(true)
    setDeleteError(null)
    try {
      await api.delete<{ message: string }>('/api/user', { password: deletePassword })
      useAuthStore.getState().logout()
      navigate('/login')
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : 'delete failed')
      setDeleting(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <Section title="Soul & Memory">
        <p className="text-sm" style={{ color: 'var(--muted)' }}>
          Your soul (personality) and memory now live as editable Markdown files in your workspace.
          Edit them from the{' '}
          <a
            href="/settings/workspace?path=SOUL.md"
            className="underline"
            style={{ color: 'var(--accent)' }}
          >
            Workspace
          </a>{' '}
          page.
        </p>
      </Section>

      {profile && (
        <Section title="Account">
          <Field label="Email" value={profile.email} />
          <Field label="User ID" value={profile.user_id} mono />
          <Field label="Role" value={profile.is_admin ? 'Admin' : 'User'} />
          <div className="flex flex-col gap-1 pt-1">
            <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
              Display Name
            </label>
            <div className="flex gap-2">
              <input
                type="text"
                value={displayName}
                onChange={e => setDisplayName(e.target.value)}
                placeholder="Your name (shown in chat and to the agent)"
                className="flex-1 rounded-lg px-3 py-2 text-sm outline-none border"
                style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
              />
              <SaveButton onClick={saveDisplayName} loading={saving} label="Save" />
            </div>
          </div>
        </Section>
      )}

      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}

      <Section title="Session">
        <button
          onClick={() => { logout(); navigate('/login', { replace: true }) }}
          className="self-start px-4 py-1.5 rounded-lg text-xs font-semibold uppercase tracking-wider transition-colors"
          style={{ color: '#ef4444', border: '1px solid #ef4444', background: 'transparent' }}
          onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = 'rgba(239,68,68,0.1)' }}
          onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = 'transparent' }}
        >
          Log Out
        </button>
      </Section>

      <Section title="Danger Zone">
        <p className="text-sm" style={{ color: 'var(--muted)' }}>
          Permanently delete your account. This removes all sessions, messages,
          channel configurations, files, and skills. <strong>This cannot be undone.</strong>
        </p>
        <button
          onClick={() => {
            setDeletePassword('')
            setDeleteError(null)
            setDeleteOpen(true)
          }}
          className="mt-3 px-4 py-2 rounded font-medium"
          style={{ background: '#ef4444', color: 'white' }}
        >
          Delete Account
        </button>
      </Section>

      {deleteOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center"
          style={{ background: 'rgba(0,0,0,0.5)' }}
          onClick={() => !deleting && setDeleteOpen(false)}
        >
          <div
            className="rounded p-6 max-w-md w-full"
            style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
            onClick={(e) => e.stopPropagation()}
          >
            <h2 className="text-lg font-semibold mb-2" style={{ color: '#ef4444' }}>Delete Account?</h2>
            <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
              This will permanently delete your account, all messages, channels,
              files, and settings. <strong>This cannot be undone.</strong>
            </p>
            <input
              type="password"
              autoFocus
              value={deletePassword}
              onChange={(e) => setDeletePassword(e.target.value)}
              placeholder="Enter your password to confirm"
              disabled={deleting}
              className="w-full px-3 py-2 rounded text-sm font-mono mb-3"
              style={{
                background: 'var(--card)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
              }}
            />
            {deleteError && (
              <p className="text-sm mb-3" style={{ color: '#ef4444' }}>{deleteError}</p>
            )}
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setDeleteOpen(false)}
                disabled={deleting}
                className="text-sm px-3 py-1 rounded"
                style={{ border: '1px solid var(--border)' }}
              >
                Cancel
              </button>
              <button
                onClick={confirmDelete}
                disabled={deleting || !deletePassword}
                className="text-sm px-3 py-1 rounded disabled:opacity-40"
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

// ── Devices Tab ───────────────────────────────────────────────────────────────

// Fields a user can PATCH on a device config.
type EditableField = 'workspace_path' | 'shell_timeout_max' | 'ssrf_whitelist' | 'fs_policy'

// Parse the server's VALIDATION_FAILED message — shape:
//   "field errors: workspace_path=not absolute; shell_timeout_max=out of range (10-1800)"
// — into a per-field error map the UI can render inline.
function parseValidationMessage(msg: string): Partial<Record<EditableField, string>> {
  const out: Partial<Record<EditableField, string>> = {}
  const body = msg.replace(/^field errors:\s*/, '')
  for (const part of body.split(';')) {
    const trimmed = part.trim()
    if (!trimmed) continue
    const eq = trimmed.indexOf('=')
    if (eq < 0) continue
    const key = trimmed.slice(0, eq).trim() as EditableField
    const val = trimmed.slice(eq + 1).trim()
    if (key === 'workspace_path' || key === 'shell_timeout_max' || key === 'ssrf_whitelist' || key === 'fs_policy') {
      out[key] = val
    }
  }
  return out
}

function DevicesTab() {
  const [devices, setDevices] = useState<Device[]>([])
  const [tokens, setTokens] = useState<DeviceToken[]>([])
  const [newTokenName, setNewTokenName] = useState('')
  const [createdToken, setCreatedToken] = useState('')
  const [expandedDevice, setExpandedDevice] = useState<string | null>(null)
  const [msg, setMsg] = useState('')

  useEffect(() => { void refresh() }, [])

  async function refresh() {
    const [devs, toks] = await Promise.all([
      api.get<Device[]>('/api/devices'),
      api.get<DeviceToken[]>('/api/device-tokens'),
    ])
    setDevices(devs)
    setTokens(toks)
  }

  async function createToken() {
    if (!newTokenName.trim()) return
    const res = await api.post<{ token: string }>('/api/device-tokens', { device_name: newTokenName.trim() })
    setCreatedToken(res.token)
    setNewTokenName('')
    void refresh()
  }

  async function deleteToken(token: string) {
    await api.delete(`/api/device-tokens/${token}`)
    void refresh()
  }

  return (
    <div className="flex flex-col gap-6">
      <Section title="Create Device Token">
        <div className="flex gap-2">
          <input
            value={newTokenName}
            onChange={e => setNewTokenName(e.target.value)}
            placeholder="Device name (e.g. linux-devbox)"
            className="flex-1 rounded-lg px-3 py-2 text-sm outline-none border"
            style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
          />
          <SaveButton onClick={createToken} label="Create" loading={false} />
        </div>
        {createdToken && (
          <div className="mt-2 p-3 rounded-lg text-xs font-mono break-all" style={{ background: 'var(--bg)', color: 'var(--accent)', border: '1px solid var(--border)' }}>
            {createdToken}
            <p className="mt-1 text-[10px]" style={{ color: 'var(--muted)' }}>
              Copy this token now — it won't be shown again.
            </p>
          </div>
        )}
      </Section>

      {tokens.length > 0 && (
        <Section title="Device Tokens">
          {tokens.map(t => (
            <div key={t.token} className="flex items-center justify-between py-1 border-b text-xs" style={{ borderColor: 'var(--border)' }}>
              <span style={{ color: 'var(--muted)' }}>{t.device_name}</span>
              <button onClick={() => deleteToken(t.token)} className="text-red-400 hover:text-red-300 text-xs">revoke</button>
            </div>
          ))}
        </Section>
      )}

      <Section title="Connected Devices">
        {devices.length === 0 && (
          <p className="text-xs" style={{ color: 'var(--muted)' }}>No devices connected.</p>
        )}
        {devices.map(d => (
          <div key={d.device_name} className="border rounded-lg overflow-hidden mb-2" style={{ borderColor: 'var(--border)' }}>
            <button
              onClick={() => setExpandedDevice(expandedDevice === d.device_name ? null : d.device_name)}
              className="w-full flex items-center justify-between px-4 py-3 text-sm hover:bg-[#1a2332] transition-colors"
            >
              <div className="flex items-center gap-2">
                <span style={{
                  width: 8, height: 8, borderRadius: '50%',
                  background: d.status === 'online' ? '#39ff14' : '#ef4444',
                  boxShadow: d.status === 'online' ? '0 0 6px #39ff14' : 'none',
                  display: 'inline-block',
                }} />
                <span style={{ color: 'var(--text)' }}>{d.device_name}</span>
                <span style={{ color: 'var(--muted)' }}>({d.tools_count} tools)</span>
              </div>
              <span style={{ color: 'var(--muted)', fontSize: 10 }}>
                {expandedDevice === d.device_name ? '▲' : '▼'}
              </span>
            </button>

            {expandedDevice === d.device_name && (
              <DeviceConfigEditor
                deviceName={d.device_name}
                onSaved={(m) => setMsg(m)}
              />
            )}
          </div>
        ))}
      </Section>

      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Device config editor (per-row expander) ───────────────────────────────────

function DeviceConfigEditor({
  deviceName,
  onSaved,
}: {
  deviceName: string
  onSaved: (msg: string) => void
}) {
  const [server, setServer] = useState<DeviceConfig | null>(null)
  const [draft, setDraft] = useState<DeviceConfig | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [fieldErrors, setFieldErrors] = useState<Partial<Record<EditableField, string>>>({})
  const [topError, setTopError] = useState<string | null>(null)
  const [confirmUnrestricted, setConfirmUnrestricted] = useState(false)

  // MCP JSON editor — unchanged from old flow.
  const [mcpJson, setMcpJson] = useState<string>('[]')
  const [mcpMsg, setMcpMsg] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const cfg = await api.get<DeviceConfig>(`/api/devices/${deviceName}/config`)
        if (cancelled) return
        setServer(cfg)
        setDraft(cfg)
        setMcpJson(JSON.stringify(cfg.mcp_servers ?? [], null, 2))
      } catch (e) {
        if (cancelled) return
        setLoadError(e instanceof Error ? e.message : 'Failed to load config')
      }
    })()
    return () => { cancelled = true }
  }, [deviceName])

  // Shallow per-field diff — only send what actually changed.
  const diff = useMemo(() => {
    if (!server || !draft) return {} as Partial<Record<EditableField, unknown>>
    const out: Partial<Record<EditableField, unknown>> = {}
    if (draft.workspace_path !== server.workspace_path) out.workspace_path = draft.workspace_path
    if (draft.shell_timeout_max !== server.shell_timeout_max) out.shell_timeout_max = draft.shell_timeout_max
    if (JSON.stringify(draft.ssrf_whitelist) !== JSON.stringify(server.ssrf_whitelist)) out.ssrf_whitelist = draft.ssrf_whitelist
    if (draft.fs_policy.mode !== server.fs_policy.mode) out.fs_policy = draft.fs_policy
    return out
  }, [server, draft])

  const hasChanges = Object.keys(diff).length > 0
  const dangerousFlip = draft != null
    && server != null
    && draft.fs_policy.mode === 'unrestricted'
    && server.fs_policy.mode !== 'unrestricted'

  async function doSave() {
    if (!hasChanges) return
    setSaving(true)
    setTopError(null)
    setFieldErrors({})
    try {
      const updated = await api.patch<DeviceConfig>(`/api/devices/${deviceName}/config`, diff)
      setServer(updated)
      setDraft(updated)
      setMcpJson(JSON.stringify(updated.mcp_servers ?? [], null, 2))
      onSaved(`Config saved for ${deviceName}`)
    } catch (e) {
      if (e instanceof ApiError && e.code === 'VALIDATION_FAILED') {
        const parsed = parseValidationMessage(e.message)
        if (Object.keys(parsed).length > 0) {
          setFieldErrors(parsed)
        } else {
          setTopError(e.message)
        }
      } else {
        setTopError(e instanceof Error ? e.message : 'Save failed')
      }
    } finally {
      setSaving(false)
    }
  }

  function handleSaveClick() {
    if (dangerousFlip) {
      setConfirmUnrestricted(true)
      return
    }
    void doSave()
  }

  function handleCancel() {
    if (!server) return
    setDraft(server)
    setFieldErrors({})
    setTopError(null)
  }

  async function saveMcp() {
    setMcpMsg(null)
    let parsed: McpServerEntry[]
    try {
      parsed = JSON.parse(mcpJson) as McpServerEntry[]
    } catch {
      setMcpMsg('Invalid JSON')
      return
    }
    try {
      await api.put(`/api/devices/${deviceName}/mcp`, { mcp_servers: parsed })
      setMcpMsg('MCP saved')
      // Pull fresh config so mcp_servers stays in sync with server state.
      const cfg = await api.get<DeviceConfig>(`/api/devices/${deviceName}/config`)
      setServer(cfg)
      setDraft(d => d ? { ...d, mcp_servers: cfg.mcp_servers } : d)
    } catch (e) {
      setMcpMsg(e instanceof Error ? e.message : 'Save failed')
    }
  }

  if (loadError) {
    return (
      <div className="px-4 py-3 border-t text-xs" style={{ borderColor: 'var(--border)', color: '#ef4444' }}>
        {loadError}
      </div>
    )
  }
  if (!draft || !server) {
    return (
      <div className="px-4 py-3 border-t text-xs" style={{ borderColor: 'var(--border)', color: 'var(--muted)' }}>
        Loading…
      </div>
    )
  }

  return (
    <div className="px-4 pb-4 flex flex-col gap-4 border-t" style={{ borderColor: 'var(--border)' }}>
      {/* workspace_path */}
      <div className="mt-3 flex flex-col gap-1">
        <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
          Workspace Path
        </label>
        <input
          type="text"
          value={draft.workspace_path}
          onChange={e => setDraft({ ...draft, workspace_path: e.target.value })}
          placeholder={`/home/<user>/.plexus/workspace/${deviceName}`}
          className="rounded-lg px-3 py-2 text-sm outline-none border font-mono"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        {fieldErrors.workspace_path && (
          <span className="text-xs" style={{ color: '#ef4444' }}>{fieldErrors.workspace_path}</span>
        )}
        <span className="text-[10px]" style={{ color: 'var(--muted)' }}>
          Absolute path on the client (must start with <code>/</code>).
        </span>
      </div>

      {/* shell_timeout_max */}
      <div className="flex flex-col gap-1">
        <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
          Shell Timeout Max (seconds)
        </label>
        <input
          type="number"
          min={10}
          max={1800}
          value={draft.shell_timeout_max}
          onChange={e => setDraft({ ...draft, shell_timeout_max: parseInt(e.target.value || '0', 10) })}
          className="rounded-lg px-3 py-2 text-sm outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        {fieldErrors.shell_timeout_max && (
          <span className="text-xs" style={{ color: '#ef4444' }}>{fieldErrors.shell_timeout_max}</span>
        )}
        <span className="text-[10px]" style={{ color: 'var(--muted)' }}>
          Cap for agent-requested timeouts; agent may pass lower. Range: 10–1800.
        </span>
      </div>

      {/* ssrf_whitelist */}
      <div className="flex flex-col gap-1">
        <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
          SSRF Whitelist (CIDRs)
        </label>
        <CidrChipInput
          values={draft.ssrf_whitelist}
          onChange={(next) => setDraft({ ...draft, ssrf_whitelist: next })}
        />
        {fieldErrors.ssrf_whitelist && (
          <span className="text-xs" style={{ color: '#ef4444' }}>{fieldErrors.ssrf_whitelist}</span>
        )}
        <span className="text-[10px]" style={{ color: 'var(--muted)' }}>
          CIDRs that punch holes in default RFC-1918 block on this device.
        </span>
      </div>

      {/* fs_policy */}
      <div className="flex flex-col gap-1">
        <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
          Filesystem Policy
        </label>
        <select
          value={draft.fs_policy.mode}
          onChange={e => setDraft({ ...draft, fs_policy: { mode: e.target.value as 'sandbox' | 'unrestricted' } })}
          className="rounded-lg px-3 py-2 text-sm outline-none border"
          style={{
            background: 'var(--bg)',
            color: draft.fs_policy.mode === 'unrestricted' ? '#ef4444' : 'var(--text)',
            borderColor: 'var(--border)',
          }}
        >
          <option value="sandbox">Sandbox (workspace only)</option>
          <option value="unrestricted">Unrestricted (full access)</option>
        </select>
        {fieldErrors.fs_policy && (
          <span className="text-xs" style={{ color: '#ef4444' }}>{fieldErrors.fs_policy}</span>
        )}
        {dangerousFlip && (
          <span className="text-[10px]" style={{ color: '#ef4444' }}>
            Requires typed confirmation when you save.
          </span>
        )}
      </div>

      {topError && (
        <p className="text-xs" style={{ color: '#ef4444' }}>{topError}</p>
      )}

      {/* Save / Cancel */}
      <div className="flex gap-2">
        <SaveButton
          onClick={handleSaveClick}
          label={hasChanges ? 'Save Config' : 'No Changes'}
          loading={saving}
        />
        <button
          onClick={handleCancel}
          disabled={!hasChanges || saving}
          className="px-4 py-1.5 rounded-lg text-xs font-semibold uppercase tracking-wider transition-colors disabled:opacity-40"
          style={{ border: '1px solid var(--border)', color: 'var(--muted)' }}
        >
          Cancel
        </button>
      </div>

      {/* MCP Servers (JSON blob — retained from old UI) */}
      <div className="pt-2">
        <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
          MCP Servers (JSON)
        </label>
        <textarea
          value={mcpJson}
          onChange={e => setMcpJson(e.target.value)}
          rows={6}
          className="mt-1 w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <div className="flex items-center gap-3">
          <SaveButton onClick={saveMcp} label="Save MCP" loading={false} />
          {mcpMsg && <span className="text-xs" style={{ color: mcpMsg.startsWith('MCP saved') ? 'var(--accent)' : '#ef4444' }}>{mcpMsg}</span>}
        </div>
      </div>

      {confirmUnrestricted && (
        <ConfirmTypedModal
          title="Allow unrestricted filesystem access?"
          warning={
            <>
              Agent will have full access to <strong>{deviceName}</strong>.
              Files outside your workspace, system files, credentials —
              all readable and writable by the agent.
            </>
          }
          match={deviceName}
          confirmLabel="Allow Unrestricted"
          busy={saving}
          onCancel={() => setConfirmUnrestricted(false)}
          onConfirm={async () => {
            setConfirmUnrestricted(false)
            await doSave()
          }}
        />
      )}
    </div>
  )
}

// ── Chip-style CIDR input ─────────────────────────────────────────────────────

function CidrChipInput({
  values,
  onChange,
}: {
  values: string[]
  onChange: (next: string[]) => void
}) {
  const [buf, setBuf] = useState('')

  function commit() {
    const v = buf.trim()
    if (!v) return
    if (values.includes(v)) { setBuf(''); return }
    onChange([...values, v])
    setBuf('')
  }

  function remove(i: number) {
    const next = values.slice()
    next.splice(i, 1)
    onChange(next)
  }

  return (
    <div
      className="rounded-lg px-2 py-1.5 border flex flex-wrap items-center gap-1"
      style={{ background: 'var(--bg)', borderColor: 'var(--border)' }}
    >
      {values.map((v, i) => (
        <span
          key={`${v}-${i}`}
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono"
          style={{ background: 'var(--card)', color: 'var(--text)', border: '1px solid var(--border)' }}
        >
          {v}
          <button
            onClick={() => remove(i)}
            className="text-red-400 hover:text-red-300"
            aria-label={`Remove ${v}`}
          >
            ×
          </button>
        </span>
      ))}
      <input
        type="text"
        value={buf}
        onChange={e => setBuf(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ',' || e.key === ' ') {
            e.preventDefault()
            commit()
          } else if (e.key === 'Backspace' && buf === '' && values.length > 0) {
            remove(values.length - 1)
          }
        }}
        onBlur={commit}
        placeholder={values.length === 0 ? '10.0.0.0/8' : ''}
        className="flex-1 min-w-[8rem] bg-transparent outline-none text-sm font-mono py-0.5"
        style={{ color: 'var(--text)' }}
      />
    </div>
  )
}

// ── Typed-confirmation modal (reusable Danger Zone pattern) ───────────────────

function ConfirmTypedModal({
  title,
  warning,
  match,
  confirmLabel,
  busy,
  onCancel,
  onConfirm,
}: {
  title: string
  warning: React.ReactNode
  match: string
  confirmLabel: string
  busy: boolean
  onCancel: () => void
  onConfirm: () => void
}) {
  const [typed, setTyped] = useState('')
  const confirmed = typed === match

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={() => !busy && onCancel()}
    >
      <div
        className="rounded p-6 max-w-md w-full"
        style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold mb-2" style={{ color: '#ef4444' }}>{title}</h2>
        <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
          {warning}
        </p>
        <p className="text-xs mb-2" style={{ color: 'var(--muted)' }}>
          Type <code style={{ color: 'var(--text)' }}>{match}</code> to confirm.
        </p>
        <input
          type="text"
          autoFocus
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          disabled={busy}
          className="w-full px-3 py-2 rounded text-sm font-mono mb-3"
          style={{
            background: 'var(--card)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
          }}
        />
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            disabled={busy}
            className="text-sm px-3 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={busy || !confirmed}
            className="text-sm px-3 py-1 rounded disabled:opacity-40"
            style={{ background: '#ef4444', color: 'white' }}
          >
            {busy ? 'Working…' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Channels Tab ──────────────────────────────────────────────────────────────

function ChannelsTab() {
  const [discord, setDiscord] = useState<DiscordConfig | null>(null)
  const [dcForm, setDcForm] = useState({ bot_token: '', partner_discord_id: '', allowed_users: '' })
  const [telegram, setTelegram] = useState<TelegramConfig | null>(null)
  const [tgForm, setTgForm] = useState({
    bot_token: '',
    partner_telegram_id: '',
    allowed_users: '',
    group_policy: 'mention' as 'mention' | 'all',
  })
  const [msg, setMsg] = useState('')

  useEffect(() => {
    api.get<DiscordConfig>('/api/discord-config').then(setDiscord).catch(() => {})
    api.get<TelegramConfig>('/api/telegram-config').then(setTelegram).catch(() => {})
  }, [])

  async function saveDiscord() {
    try {
      await api.post('/api/discord-config', {
        bot_token: dcForm.bot_token,
        partner_discord_id: dcForm.partner_discord_id,
        allowed_users: dcForm.allowed_users.split(',').map(s => s.trim()).filter(Boolean),
      })
      setMsg('Discord saved.')
    } catch (e) { setMsg(e instanceof Error ? e.message : 'Error') }
  }

  async function deleteDiscord() {
    await api.delete('/api/discord-config')
    setDiscord(null)
    setMsg('Discord removed.')
  }

  async function saveTelegram() {
    try {
      await api.post('/api/telegram-config', {
        bot_token: tgForm.bot_token,
        partner_telegram_id: tgForm.partner_telegram_id,
        allowed_users: tgForm.allowed_users.split(',').map(s => s.trim()).filter(Boolean),
        group_policy: tgForm.group_policy,
      })
      setMsg('Telegram saved.')
    } catch (e) { setMsg(e instanceof Error ? e.message : 'Error') }
  }

  async function deleteTelegram() {
    await api.delete('/api/telegram-config')
    setTelegram(null)
    setMsg('Telegram removed.')
  }

  return (
    <div className="flex flex-col gap-6">
      <Section title="Discord">
        {discord && (
          <div className="mb-3 p-3 rounded-lg text-xs" style={{ background: 'var(--card)', border: '1px solid var(--border)' }}>
            <p style={{ color: 'var(--muted)' }}>Bot user: <span style={{ color: 'var(--text)' }}>{discord.bot_user_id}</span></p>
            <p style={{ color: 'var(--muted)' }}>Partner: <span style={{ color: 'var(--text)' }}>{discord.partner_discord_id}</span></p>
            <button onClick={deleteDiscord} className="mt-2 text-red-400 hover:text-red-300 text-xs">Remove</button>
          </div>
        )}
        <FormField label="Bot Token" value={dcForm.bot_token} onChange={v => setDcForm(f => ({ ...f, bot_token: v }))} type="password" />
        <FormField label="Owner Discord ID" value={dcForm.partner_discord_id} onChange={v => setDcForm(f => ({ ...f, partner_discord_id: v }))} />
        <FormField label="Allowed Users (comma-separated IDs)" value={dcForm.allowed_users} onChange={v => setDcForm(f => ({ ...f, allowed_users: v }))} />
        <SaveButton onClick={saveDiscord} loading={false} />
      </Section>

      <Section title="Telegram">
        {telegram && (
          <div className="mb-3 p-3 rounded-lg text-xs" style={{ background: 'var(--card)', border: '1px solid var(--border)' }}>
            <p style={{ color: 'var(--muted)' }}>Partner: <span style={{ color: 'var(--text)' }}>{telegram.partner_telegram_id}</span></p>
            <button onClick={deleteTelegram} className="mt-2 text-red-400 hover:text-red-300 text-xs">Remove</button>
          </div>
        )}
        <FormField label="Bot Token" value={tgForm.bot_token} onChange={v => setTgForm(f => ({ ...f, bot_token: v }))} type="password" />
        <FormField label="Owner Telegram ID" value={tgForm.partner_telegram_id} onChange={v => setTgForm(f => ({ ...f, partner_telegram_id: v }))} />
        <FormField label="Allowed Users (comma-separated)" value={tgForm.allowed_users} onChange={v => setTgForm(f => ({ ...f, allowed_users: v }))} />
        <div className="flex flex-col gap-1">
          <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>Group Policy</label>
          <select
            value={tgForm.group_policy}
            onChange={e => setTgForm(f => ({ ...f, group_policy: e.target.value as 'mention' | 'all' }))}
            className="rounded-lg px-3 py-2 text-sm outline-none border"
            style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
          >
            <option value="mention">Respond when mentioned</option>
            <option value="all">Respond to all messages</option>
          </select>
        </div>
        <SaveButton onClick={saveTelegram} loading={false} />
      </Section>

      {msg && <p className="text-xs mt-2" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Skills Tab ────────────────────────────────────────────────────────────────

function SkillsTab() {
  const [skills, setSkills] = useState<WorkspaceSkill[]>([])
  const [msg, setMsg] = useState<string | null>(null)

  useEffect(() => {
    void load()
  }, [])

  async function load() {
    try {
      const data = await api.get<WorkspaceSkill[]>('/api/workspace/skills')
      setSkills(data)
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'load failed')
    }
  }

  return (
    <Section title="Skills">
      <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
        Skills are Markdown files at <code>skills/{'{name}'}/SKILL.md</code> in your workspace.
        Create, edit, or delete them from the{' '}
        <a
          href="/settings/workspace?path=skills"
          className="underline"
          style={{ color: 'var(--accent)' }}
        >
          Workspace
        </a>{' '}
        page.
      </p>
      {msg && (
        <div className="text-sm" style={{ color: '#ef4444' }}>
          {msg}
        </div>
      )}
      <ul className="list-none p-0">
        {skills.map((s) => (
          <li
            key={s.name}
            className="flex items-center gap-2 py-1 border-b"
            style={{ borderColor: 'var(--border)' }}
          >
            <strong>{s.name}</strong>
            {s.always_on && <span className="text-xs" style={{ color: 'var(--accent)' }}>always-on</span>}
            <span className="text-sm flex-1" style={{ color: 'var(--muted)' }}>{s.description}</span>
            <a
              href={`/settings/workspace?path=skills/${s.name}/SKILL.md`}
              className="text-xs underline"
              style={{ color: 'var(--accent)' }}
            >
              Edit
            </a>
          </li>
        ))}
      </ul>
    </Section>
  )
}

// ── Cron Tab ──────────────────────────────────────────────────────────────────

function CronTab() {
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [form, setForm] = useState({
    message: '',
    name: '',
    scheduleType: 'cron_expr' as 'cron_expr' | 'every_seconds' | 'at',
    cron_expr: '',
    every_seconds: '',
    at: '',
    timezone: 'UTC',
    channel: 'gateway',
  })
  const [msg, setMsg] = useState('')

  useEffect(() => { void loadJobs() }, [])

  async function loadJobs() {
    const j = await api.get<{ cron_jobs: CronJob[] }>('/api/cron-jobs')
    setJobs(j.cron_jobs)
  }

  async function createJob() {
    try {
      const body: Record<string, unknown> = {
        message: form.message,
        name: form.name || undefined,
        timezone: form.timezone,
        channel: form.channel,
      }
      if (form.scheduleType === 'cron_expr') body['cron_expr'] = form.cron_expr
      if (form.scheduleType === 'every_seconds') body['every_seconds'] = parseInt(form.every_seconds)
      if (form.scheduleType === 'at') body['at'] = form.at
      await api.post('/api/cron-jobs', body)
      setMsg('Job created.')
      void loadJobs()
    } catch (e) { setMsg(e instanceof Error ? e.message : 'Error') }
  }

  async function toggleJob(job: CronJob) {
    await api.patch(`/api/cron-jobs/${job.job_id}`, { enabled: !job.enabled })
    void loadJobs()
  }

  async function deleteJob(jobId: string) {
    await api.delete(`/api/cron-jobs/${jobId}`)
    void loadJobs()
  }

  return (
    <div className="flex flex-col gap-6">
      <Section title="Scheduled Jobs">
        {jobs.length === 0 && (
          <p className="text-xs" style={{ color: 'var(--muted)' }}>No cron jobs.</p>
        )}
        {jobs.map(j => (
          <div key={j.job_id} className="flex items-start justify-between py-2 border-b text-xs" style={{ borderColor: 'var(--border)' }}>
            <div>
              <p style={{ color: 'var(--text)' }}>{j.name ?? j.job_id.slice(0, 8)}</p>
              <p style={{ color: 'var(--muted)' }}>
                {j.cron_expr ?? (j.every_seconds ? `every ${j.every_seconds}s` : j.at)}
              </p>
              <p style={{ color: 'var(--muted)' }} className="truncate max-w-xs">{j.message}</p>
            </div>
            <div className="flex gap-2 ml-4 shrink-0">
              <button
                onClick={() => toggleJob(j)}
                className="text-xs"
                style={{ color: j.enabled ? 'var(--accent)' : 'var(--muted)' }}
              >
                {j.enabled ? 'enabled' : 'disabled'}
              </button>
              <button onClick={() => deleteJob(j.job_id)} className="text-xs text-red-400 hover:text-red-300">
                delete
              </button>
            </div>
          </div>
        ))}
      </Section>

      <Section title="New Job">
        <FormField label="Message (what the agent should do)" value={form.message} onChange={v => setForm(f => ({ ...f, message: v }))} />
        <FormField label="Name (optional)" value={form.name} onChange={v => setForm(f => ({ ...f, name: v }))} />
        <FormField label="Channel (gateway / discord / telegram)" value={form.channel} onChange={v => setForm(f => ({ ...f, channel: v }))} />
        <FormField label="Timezone (e.g. UTC, America/New_York)" value={form.timezone} onChange={v => setForm(f => ({ ...f, timezone: v }))} />

        <div className="flex flex-col gap-1">
          <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>Schedule Type</label>
          <div className="flex gap-4">
            {(['cron_expr', 'every_seconds', 'at'] as const).map(t => (
              <label key={t} className="flex items-center gap-1 text-xs cursor-pointer" style={{ color: form.scheduleType === t ? 'var(--accent)' : 'var(--muted)' }}>
                <input
                  type="radio"
                  checked={form.scheduleType === t}
                  onChange={() => setForm(f => ({ ...f, scheduleType: t }))}
                />
                {t === 'cron_expr' ? 'Cron expression' : t === 'every_seconds' ? 'Every N seconds' : 'One-shot (at)'}
              </label>
            ))}
          </div>
        </div>

        {form.scheduleType === 'cron_expr' && (
          <FormField label="Cron expression (e.g. 0 9 * * 1-5)" value={form.cron_expr} onChange={v => setForm(f => ({ ...f, cron_expr: v }))} />
        )}
        {form.scheduleType === 'every_seconds' && (
          <FormField label="Interval (seconds)" value={form.every_seconds} onChange={v => setForm(f => ({ ...f, every_seconds: v }))} type="number" />
        )}
        {form.scheduleType === 'at' && (
          <FormField label="Run at (ISO datetime)" value={form.at} onChange={v => setForm(f => ({ ...f, at: v }))} type="datetime-local" />
        )}

        <SaveButton onClick={createJob} label="Create Job" loading={false} />
      </Section>

      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Shared UI primitives ──────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-xs uppercase tracking-widest font-semibold" style={{ color: 'var(--muted)' }}>
        {title}
      </h2>
      {children}
    </div>
  )
}

function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between text-sm py-1 border-b" style={{ borderColor: 'var(--border)' }}>
      <span style={{ color: 'var(--muted)' }}>{label}</span>
      <span style={{ color: 'var(--text)', fontFamily: mono ? 'monospace' : undefined }}>{value}</span>
    </div>
  )
}

function FormField({
  label,
  value,
  onChange,
  type = 'text',
}: {
  label: string
  value: string
  onChange: (v: string) => void
  type?: string
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>{label}</label>
      <input
        type={type}
        value={value}
        onChange={e => onChange(e.target.value)}
        className="rounded-lg px-3 py-2 text-sm outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
    </div>
  )
}

function SaveButton({
  onClick,
  label = 'Save',
  loading,
}: {
  onClick: () => void
  label?: string
  loading: boolean
}) {
  return (
    <button
      onClick={onClick}
      disabled={loading}
      className="self-start px-4 py-1.5 rounded-lg text-xs font-semibold uppercase tracking-wider transition-all disabled:opacity-50"
      style={{ background: 'var(--accent)', color: '#000', boxShadow: loading ? 'none' : '0 0 8px var(--accent)' }}
    >
      {loading ? 'Saving…' : label}
    </button>
  )
}
