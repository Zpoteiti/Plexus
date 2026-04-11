// Stub — will be fully implemented in Task 6
import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { wsManager } from '../lib/ws'

interface AuthState {
  token: string | null
  userId: string | null
  isAdmin: boolean
  login: (email: string, password: string) => Promise<void>
  logout: () => void
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      token: null,
      userId: null,
      isAdmin: false,
      login: async (email, password) => {
        const res = await fetch('/api/auth/login', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ email, password }),
        })
        if (!res.ok) {
          const body = await res.json().catch(() => ({})) as Record<string, unknown>
          const errObj = body?.['error'] as Record<string, unknown> | undefined
          const msg = errObj?.['message']
          throw new Error(typeof msg === 'string' ? msg : 'Login failed')
        }
        const data = await res.json() as { token: string; user_id: string; is_admin: boolean }
        set({ token: data.token, userId: data.user_id, isAdmin: data.is_admin })
      },
      logout: () => {
        wsManager.disconnect()
        set({ token: null, userId: null, isAdmin: false })
      },
    }),
    {
      name: 'plexus-auth',
      partialize: (s) => ({ token: s.token, userId: s.userId, isAdmin: s.isAdmin }),
    },
  ),
)
