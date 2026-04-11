import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { api } from '../lib/api'
import { useAuthStore } from '../store/auth'
import type {
  User,
  Device,
  DeviceToken,
  DevicePolicy,
  McpServerEntry,
  DiscordConfig,
  TelegramConfig,
  Skill,
  CronJob,
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
  const [soul, setSoul] = useState('')
  const [memory, setMemory] = useState('')
  const [saving, setSaving] = useState(false)
  const [msg, setMsg] = useState('')
  const logout = useAuthStore(s => s.logout)
  const refreshProfile = useAuthStore(s => s.refreshProfile)
  const navigate = useNavigate()

  useEffect(() => {
    void (async () => {
      const [p, s, m] = await Promise.all([
        api.get<User>('/api/user/profile'),
        api.get<{ soul: string }>('/api/user/soul'),
        api.get<{ memory: string }>('/api/user/memory'),
      ])
      setProfile(p)
      setDisplayName(p.display_name ?? '')
      setSoul(s.soul ?? '')
      setMemory(m.memory ?? '')
    })()
  }, [])

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

  async function saveSoul() {
    setSaving(true)
    try {
      await api.patch('/api/user/soul', { soul })
      setMsg('Soul saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSaving(false)
    }
  }

  async function saveMemory() {
    setSaving(true)
    try {
      await api.patch('/api/user/memory', { memory })
      setMsg('Memory saved.')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
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
                placeholder="e.g. Bob"
                className="flex-1 rounded-lg px-3 py-2 text-sm outline-none border"
                style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
              />
              <SaveButton onClick={saveDisplayName} loading={saving} label="Save" />
            </div>
          </div>
        </Section>
      )}

      <Section title="Soul (system prompt personality)">
        <textarea
          value={soul}
          onChange={e => setSoul(e.target.value)}
          rows={6}
          className="w-full rounded-lg p-3 text-sm font-mono resize-y outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <SaveButton onClick={saveSoul} loading={saving} />
      </Section>

      <Section title="Memory (persisted agent context)">
        <div className="text-xs mb-1" style={{ color: 'var(--muted)' }}>
          {memory.length} / 4000 characters
        </div>
        <textarea
          value={memory}
          onChange={e => setMemory(e.target.value)}
          rows={6}
          maxLength={4000}
          className="w-full rounded-lg p-3 text-sm font-mono resize-y outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <SaveButton onClick={saveMemory} loading={saving} />
      </Section>

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
    </div>
  )
}

// ── Devices Tab ───────────────────────────────────────────────────────────────

function DevicesTab() {
  const [devices, setDevices] = useState<Device[]>([])
  const [tokens, setTokens] = useState<DeviceToken[]>([])
  const [newTokenName, setNewTokenName] = useState('')
  const [createdToken, setCreatedToken] = useState('')
  const [expandedDevice, setExpandedDevice] = useState<string | null>(null)
  const [policies, setPolicies] = useState<Record<string, DevicePolicy>>({})
  const [mcpConfigs, setMcpConfigs] = useState<Record<string, string>>({})
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

  async function expandDevice(name: string) {
    setExpandedDevice(expandedDevice === name ? null : name)
    if (!policies[name]) {
      const [policy, mcp] = await Promise.all([
        api.get<DevicePolicy>(`/api/devices/${name}/policy`),
        api.get<{ mcp_servers: McpServerEntry[] }>(`/api/devices/${name}/mcp`),
      ])
      setPolicies(p => ({ ...p, [name]: policy }))
      setMcpConfigs(m => ({ ...m, [name]: JSON.stringify(mcp.mcp_servers, null, 2) }))
    }
  }

  async function savePolicy(name: string) {
    const policy = policies[name]
    if (!policy) return
    await api.patch(`/api/devices/${name}/policy`, { fs_policy: policy.fs_policy })
    setMsg(`Policy saved for ${name}`)
  }

  async function saveMcp(name: string) {
    try {
      const parsed = JSON.parse(mcpConfigs[name] ?? '[]') as McpServerEntry[]
      await api.put(`/api/devices/${name}/mcp`, { mcp_servers: parsed })
      setMsg(`MCP saved for ${name}`)
    } catch {
      setMsg('Invalid JSON')
    }
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
              onClick={() => expandDevice(d.device_name)}
              className="w-full flex items-center justify-between px-4 py-3 text-sm hover:bg-[#1a2332] transition-colors"
            >
              <div className="flex items-center gap-2">
                <span style={{
                  width: 8, height: 8, borderRadius: '50%',
                  background: d.online ? '#39ff14' : '#ef4444',
                  boxShadow: d.online ? '0 0 6px #39ff14' : 'none',
                  display: 'inline-block',
                }} />
                <span style={{ color: 'var(--text)' }}>{d.device_name}</span>
                <span style={{ color: 'var(--muted)' }}>({d.tool_count} tools)</span>
              </div>
              <span style={{ color: 'var(--muted)', fontSize: 10 }}>
                {expandedDevice === d.device_name ? '▲' : '▼'}
              </span>
            </button>

            {expandedDevice === d.device_name && policies[d.device_name] && (
              <div className="px-4 pb-4 flex flex-col gap-4 border-t" style={{ borderColor: 'var(--border)' }}>
                <div className="mt-3">
                  <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
                    Filesystem Policy
                  </label>
                  <select
                    value={policies[d.device_name].fs_policy.mode}
                    onChange={e => setPolicies(p => ({
                      ...p,
                      [d.device_name]: {
                        ...p[d.device_name],
                        fs_policy: { mode: e.target.value as 'sandbox' | 'unrestricted' },
                      },
                    }))}
                    className="mt-1 w-full rounded-lg px-3 py-2 text-sm outline-none border"
                    style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
                  >
                    <option value="sandbox">Sandbox (workspace only)</option>
                    <option value="unrestricted">Unrestricted (full access)</option>
                  </select>
                  <SaveButton onClick={() => savePolicy(d.device_name)} label="Save Policy" loading={false} />
                </div>

                <div>
                  <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
                    MCP Servers (JSON)
                  </label>
                  <textarea
                    value={mcpConfigs[d.device_name] ?? '[]'}
                    onChange={e => setMcpConfigs(m => ({ ...m, [d.device_name]: e.target.value }))}
                    rows={6}
                    className="mt-1 w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border"
                    style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
                  />
                  <SaveButton onClick={() => saveMcp(d.device_name)} label="Save MCP" loading={false} />
                </div>
              </div>
            )}
          </div>
        ))}
      </Section>

      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
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
  const [skills, setSkills] = useState<Skill[]>([])
  const [installRepo, setInstallRepo] = useState('')
  const [pasteContent, setPasteContent] = useState('')
  const [msg, setMsg] = useState('')

  useEffect(() => { void loadSkills() }, [])

  async function loadSkills() {
    const s = await api.get<Skill[]>('/api/skills')
    setSkills(s)
  }

  async function installFromGithub() {
    try {
      await api.post('/api/skills/install', { repo: installRepo.trim() })
      setInstallRepo('')
      setMsg('Skill installed.')
      void loadSkills()
    } catch (e) { setMsg(e instanceof Error ? e.message : 'Error') }
  }

  async function pasteSkill() {
    try {
      await api.post('/api/skills', { content: pasteContent })
      setPasteContent('')
      setMsg('Skill created.')
      void loadSkills()
    } catch (e) { setMsg(e instanceof Error ? e.message : 'Error') }
  }

  async function deleteSkill(name: string) {
    await api.delete(`/api/skills/${name}`)
    void loadSkills()
  }

  return (
    <div className="flex flex-col gap-6">
      <Section title="Installed Skills">
        {skills.length === 0 && (
          <p className="text-xs" style={{ color: 'var(--muted)' }}>No skills installed.</p>
        )}
        {skills.map(s => (
          <div key={s.name} className="flex items-center justify-between py-2 border-b text-sm" style={{ borderColor: 'var(--border)' }}>
            <div>
              <span style={{ color: 'var(--text)' }}>{s.name}</span>
              {s.always_on && (
                <span className="ml-2 text-xs px-1 rounded" style={{ background: 'rgba(57,255,20,0.1)', color: 'var(--accent)' }}>
                  always-on
                </span>
              )}
              <p className="text-xs mt-0.5" style={{ color: 'var(--muted)' }}>{s.description}</p>
            </div>
            <button onClick={() => deleteSkill(s.name)} className="text-red-400 hover:text-red-300 text-xs ml-4">delete</button>
          </div>
        ))}
      </Section>

      <Section title="Install from GitHub">
        <FormField label="Repository (owner/repo or full URL)" value={installRepo} onChange={setInstallRepo} />
        <SaveButton onClick={installFromGithub} label="Install" loading={false} />
      </Section>

      <Section title="Paste SKILL.md content">
        <textarea
          value={pasteContent}
          onChange={e => setPasteContent(e.target.value)}
          rows={8}
          placeholder={'---\nname: my-skill\ndescription: Does something\n---\n\n# My Skill\n...'}
          className="w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <SaveButton onClick={pasteSkill} label="Create Skill" loading={false} />
      </Section>

      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
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
    const j = await api.get<CronJob[]>('/api/cron-jobs')
    setJobs(j)
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
