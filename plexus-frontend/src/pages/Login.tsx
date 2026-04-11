import { useState, FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuthStore } from '../store/auth'

type Mode = 'login' | 'register'

const inputStyle = {
  background: 'var(--bg)',
  color: 'var(--text)',
  borderColor: 'var(--border)',
}

export default function Login() {
  const [mode, setMode] = useState<Mode>('login')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [adminToken, setAdminToken] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const login = useAuthStore(s => s.login)
  const register = useAuthStore(s => s.register)
  const navigate = useNavigate()

  function switchMode(next: Mode) {
    setMode(next)
    setError('')
    setConfirm('')
    setAdminToken('')
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError('')

    if (mode === 'register' && password !== confirm) {
      setError('Passwords do not match')
      return
    }

    setLoading(true)
    try {
      if (mode === 'login') {
        await login(email, password)
      } else {
        await register(email, password, adminToken || undefined)
      }
      navigate('/chat', { replace: true })
    } catch (err) {
      setError(err instanceof Error ? err.message : mode === 'login' ? 'Login failed' : 'Registration failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div
      className="min-h-screen flex items-center justify-center"
      style={{ background: 'var(--bg)' }}
    >
      <div
        className="w-full max-w-sm p-8 rounded-xl border"
        style={{ background: 'var(--card)', borderColor: 'var(--border)' }}
      >
        {/* Logo / title */}
        <div className="mb-6 text-center">
          <span
            className="text-2xl font-bold tracking-widest uppercase"
            style={{ color: 'var(--accent)', textShadow: '0 0 12px var(--accent)' }}
          >
            PLEXUS
          </span>
          <p className="mt-1 text-xs" style={{ color: 'var(--muted)' }}>
            Distributed AI Agent System
          </p>
        </div>

        {/* Tabs */}
        <div className="flex mb-6 border-b" style={{ borderColor: 'var(--border)' }}>
          {(['login', 'register'] as Mode[]).map(m => (
            <button
              key={m}
              type="button"
              onClick={() => switchMode(m)}
              className="flex-1 pb-2 text-xs uppercase tracking-wider font-semibold transition-colors cursor-pointer"
              style={{
                color: mode === m ? 'var(--accent)' : 'var(--muted)',
                borderBottom: mode === m ? '2px solid var(--accent)' : '2px solid transparent',
                marginBottom: '-1px',
                background: 'transparent',
                border: 'none',
              }}
            >
              {m === 'login' ? 'Sign In' : 'Register'}
            </button>
          ))}
        </div>

        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1">
            <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
              Email
            </label>
            <input
              type="email"
              value={email}
              onChange={e => setEmail(e.target.value)}
              required
              autoFocus
              className="w-full rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
              style={inputStyle}
            />
          </div>

          <div className="flex flex-col gap-1">
            <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
              Password
            </label>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              required
              className="w-full rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
              style={inputStyle}
            />
          </div>

          {mode === 'register' && (
            <>
              <div className="flex flex-col gap-1">
                <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
                  Confirm Password
                </label>
                <input
                  type="password"
                  value={confirm}
                  onChange={e => setConfirm(e.target.value)}
                  required
                  className="w-full rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
                  style={inputStyle}
                />
              </div>

              <div className="flex flex-col gap-1">
                <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>
                  Admin Token <span style={{ color: 'var(--muted)', fontWeight: 'normal' }}>(optional)</span>
                </label>
                <input
                  type="password"
                  value={adminToken}
                  onChange={e => setAdminToken(e.target.value)}
                  placeholder="Leave blank for regular account"
                  className="w-full rounded-lg px-3 py-2 text-sm outline-none border focus:border-[#39ff14] transition-colors"
                  style={{ ...inputStyle, borderStyle: 'dashed' }}
                />
              </div>
            </>
          )}

          {error && (
            <p className="text-xs text-red-400">{error}</p>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full rounded-lg py-2 text-sm font-semibold tracking-wider uppercase transition-all disabled:opacity-50 cursor-pointer"
            style={{
              background: 'var(--accent)',
              color: '#000',
              boxShadow: loading ? 'none' : '0 0 12px var(--accent)',
            }}
          >
            {loading
              ? mode === 'login' ? 'Signing in…' : 'Registering…'
              : mode === 'login' ? 'Sign In' : 'Create Account'}
          </button>
        </form>
      </div>
    </div>
  )
}
