import { useState, useEffect, FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { api } from '../lib/api'
import type { LlmConfig, McpServerEntry } from '../lib/types'

// ── Types ─────────────────────────────────────────────────────────────────────

interface Step {
  id: number
  title: string
  subtitle: string
}

const STEPS: Step[] = [
  { id: 1, title: 'LLM Provider', subtitle: 'Connect to any OpenAI-compatible API endpoint.' },
  { id: 2, title: 'Default Soul', subtitle: 'A system prompt applied to all users by default.' },
  { id: 3, title: 'Rate Limits', subtitle: 'Cap messages per minute per user. 0 = unlimited.' },
  { id: 4, title: 'Server MCP', subtitle: 'Shared MCP tool servers available to all users.' },
]

const WIZARD_KEY = 'plexus-wizard-done'

function markDone() {
  localStorage.setItem(WIZARD_KEY, 'true')
}

// ── Shell ──────────────────────────────────────────────────────────────────────

export default function Wizard() {
  const [step, setStep] = useState(0) // 0-indexed
  const navigate = useNavigate()

  function finish() {
    markDone()
    navigate('/chat', { replace: true })
  }

  function next() {
    if (step < STEPS.length - 1) setStep(s => s + 1)
    else finish()
  }

  const current = STEPS[step]

  return (
    <div className="min-h-screen flex items-center justify-center" style={{ background: 'var(--bg)' }}>
      <div className="w-full max-w-lg px-8 py-10 rounded-xl border" style={{ background: 'var(--card)', borderColor: 'var(--border)' }}>

        {/* Header */}
        <div className="flex items-start justify-between mb-8">
          <div>
            <span className="text-2xl font-bold tracking-widest uppercase" style={{ color: 'var(--accent)', textShadow: '0 0 12px var(--accent)' }}>
              PLEXUS
            </span>
            <p className="mt-1 text-xs" style={{ color: 'var(--muted)' }}>Setup &amp; Configuration</p>
          </div>
          <button
            onClick={finish}
            className="text-xs px-3 py-1.5 rounded-lg border transition-colors hover:border-[var(--accent)]"
            style={{ color: 'var(--muted)', borderColor: 'var(--border)', background: 'transparent' }}
          >
            Skip all
          </button>
        </div>

        {/* Progress dots */}
        <div className="flex items-center gap-2 mb-8">
          {STEPS.map((s, i) => (
            <div key={s.id} className="flex items-center gap-2">
              <div
                style={{
                  width: i === step ? 20 : 8,
                  height: 8,
                  borderRadius: 4,
                  background: i < step ? 'var(--accent)' : i === step ? 'var(--accent)' : 'var(--border)',
                  opacity: i < step ? 0.4 : 1,
                  transition: 'all 0.2s',
                }}
              />
            </div>
          ))}
          <span className="ml-2 text-xs" style={{ color: 'var(--muted)' }}>
            Step {step + 1} of {STEPS.length}
          </span>
        </div>

        {/* Step title */}
        <div className="mb-6">
          <h2 className="text-base font-semibold" style={{ color: 'var(--text)' }}>{current.title}</h2>
          <p className="mt-1 text-xs" style={{ color: 'var(--muted)' }}>{current.subtitle}</p>
        </div>

        {/* Step content */}
        {step === 0 && <LlmStep onNext={next} onSkip={next} />}
        {step === 1 && <SoulStep onNext={next} onSkip={next} />}
        {step === 2 && <RateLimitStep onNext={next} onSkip={next} />}
        {step === 3 && <McpStep onNext={finish} onSkip={finish} isLast />}
      </div>
    </div>
  )
}

// ── Step: LLM ─────────────────────────────────────────────────────────────────

function LlmStep({ onNext, onSkip }: { onNext: () => void; onSkip: () => void }) {
  const [form, setForm] = useState<LlmConfig>({ api_base: '', model: '', api_key: '', context_window: 128000 })
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<LlmConfig | { status: string }>('/api/llm-config')
      .then(r => { if (!('status' in r)) setForm(r as LlmConfig) })
      .catch(() => {})
  }, [])

  async function handleSave(e: FormEvent) {
    e.preventDefault()
    setLoading(true)
    try {
      await api.put('/api/llm-config', form)
      onNext()
    } catch (err) {
      setMsg(err instanceof Error ? err.message : 'Save failed')
      setLoading(false)
    }
  }

  return (
    <form onSubmit={handleSave} className="flex flex-col gap-4">
      <WizardField label="API Base URL" value={form.api_base} onChange={v => setForm(f => ({ ...f, api_base: v }))} placeholder="https://api.openai.com/v1" />
      <WizardField label="Model" value={form.model} onChange={v => setForm(f => ({ ...f, model: v }))} placeholder="gpt-4o" />
      <WizardField label="API Key" value={form.api_key} onChange={v => setForm(f => ({ ...f, api_key: v }))} type="password" placeholder="sk-..." />
      <WizardField label="Context Window (tokens)" value={String(form.context_window)} onChange={v => setForm(f => ({ ...f, context_window: parseInt(v) || 128000 }))} type="number" />
      {msg && <p className="text-xs text-red-400">{msg}</p>}
      <StepButtons loading={loading} onSkip={onSkip} saveLabel="Save & Continue" />
    </form>
  )
}

// ── Step: Soul ────────────────────────────────────────────────────────────────

function SoulStep({ onNext, onSkip }: { onNext: () => void; onSkip: () => void }) {
  const [soul, setSoul] = useState('')
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<{ soul: string }>('/api/admin/default-soul').then(r => setSoul(r.soul ?? '')).catch(() => {})
  }, [])

  async function handleSave(e: FormEvent) {
    e.preventDefault()
    setLoading(true)
    try {
      await api.put('/api/admin/default-soul', { soul })
      onNext()
    } catch (err) {
      setMsg(err instanceof Error ? err.message : 'Save failed')
      setLoading(false)
    }
  }

  return (
    <form onSubmit={handleSave} className="flex flex-col gap-4">
      <textarea
        value={soul}
        onChange={e => setSoul(e.target.value)}
        rows={8}
        placeholder="You are a helpful AI assistant running on Plexus..."
        className="w-full rounded-lg p-3 text-sm font-mono resize-y outline-none border focus:border-[#39ff14] transition-colors"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
      {msg && <p className="text-xs text-red-400">{msg}</p>}
      <StepButtons loading={loading} onSkip={onSkip} saveLabel="Save & Continue" />
    </form>
  )
}

// ── Step: Rate Limit ──────────────────────────────────────────────────────────

function RateLimitStep({ onNext, onSkip }: { onNext: () => void; onSkip: () => void }) {
  const [rateLimit, setRateLimit] = useState(0)
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<{ rate_limit_per_min: number }>('/api/admin/rate-limit')
      .then(r => setRateLimit(r.rate_limit_per_min))
      .catch(() => {})
  }, [])

  async function handleSave(e: FormEvent) {
    e.preventDefault()
    setLoading(true)
    try {
      await api.put('/api/admin/rate-limit', { rate_limit_per_min: rateLimit })
      onNext()
    } catch (err) {
      setMsg(err instanceof Error ? err.message : 'Save failed')
      setLoading(false)
    }
  }

  return (
    <form onSubmit={handleSave} className="flex flex-col gap-4">
      <div className="flex items-center gap-4">
        <input
          type="number"
          min={0}
          value={rateLimit}
          onChange={e => setRateLimit(parseInt(e.target.value) || 0)}
          className="w-32 rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
          style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        />
        <span className="text-sm" style={{ color: 'var(--muted)' }}>messages / minute &nbsp;<span style={{ color: 'var(--text)' }}>(0 = unlimited)</span></span>
      </div>
      {msg && <p className="text-xs text-red-400">{msg}</p>}
      <StepButtons loading={loading} onSkip={onSkip} saveLabel="Save & Continue" />
    </form>
  )
}

// ── Step: Server MCP ──────────────────────────────────────────────────────────

function McpStep({ onNext, onSkip, isLast }: { onNext: () => void; onSkip: () => void; isLast?: boolean }) {
  const [json, setJson] = useState('[]')
  const [msg, setMsg] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    api.get<{ mcp_servers: McpServerEntry[] }>('/api/server-mcp')
      .then(r => setJson(JSON.stringify(r.mcp_servers, null, 2)))
      .catch(() => {})
  }, [])

  async function handleSave(e: FormEvent) {
    e.preventDefault()
    setLoading(true)
    try {
      const parsed = JSON.parse(json) as McpServerEntry[]
      await api.put('/api/server-mcp', { mcp_servers: parsed })
      onNext()
    } catch (err) {
      setMsg(err instanceof Error ? err.message : 'Invalid JSON')
      setLoading(false)
    }
  }

  return (
    <form onSubmit={handleSave} className="flex flex-col gap-4">
      <textarea
        value={json}
        onChange={e => setJson(e.target.value)}
        rows={10}
        placeholder={'[\n  {\n    "name": "my-mcp",\n    "command": "uvx",\n    "args": ["my-mcp-server"],\n    "env": {},\n    "enabled": true\n  }\n]'}
        className="w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border focus:border-[#39ff14] transition-colors"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
      {msg && <p className="text-xs text-red-400">{msg}</p>}
      <StepButtons loading={loading} onSkip={onSkip} saveLabel={isLast ? 'Save & Finish' : 'Save & Continue'} skipLabel={isLast ? 'Finish' : 'Skip'} />
    </form>
  )
}

// ── Shared primitives ─────────────────────────────────────────────────────────

function WizardField({ label, value, onChange, type = 'text', placeholder }: {
  label: string; value: string; onChange: (v: string) => void; type?: string; placeholder?: string
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>{label}</label>
      <input
        type={type}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        className="rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
      />
    </div>
  )
}

function StepButtons({ loading, onSkip, saveLabel, skipLabel = 'Skip' }: {
  loading: boolean; onSkip: () => void; saveLabel: string; skipLabel?: string
}) {
  return (
    <div className="flex items-center gap-3 mt-2">
      <button
        type="submit"
        disabled={loading}
        className="px-5 py-2 rounded-lg text-sm font-semibold tracking-wider uppercase transition-all disabled:opacity-50 cursor-pointer"
        style={{ background: 'var(--accent)', color: '#000', boxShadow: loading ? 'none' : '0 0 10px var(--accent)' }}
      >
        {loading ? 'Saving…' : saveLabel}
      </button>
      <button
        type="button"
        onClick={onSkip}
        className="px-4 py-2 rounded-lg text-sm transition-colors cursor-pointer"
        style={{ color: 'var(--muted)', background: 'transparent' }}
      >
        {skipLabel}
      </button>
    </div>
  )
}

export { WIZARD_KEY }
