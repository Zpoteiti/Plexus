// Stub — will be fully implemented in Task 6
import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { wsManager } from '../lib/ws'

interface AuthState {
  token: string | null
  userId: string | null
  isAdmin: boolean
  displayName: string | null
  login: (email: string, password: string) => Promise<void>
  register: (email: string, password: string, adminToken?: string) => Promise<void>
  logout: () => void
  refreshProfile: () => Promise<void>
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      token: null,
      userId: null,
      isAdmin: false,
      displayName: null,
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
      register: async (email, password, adminToken) => {
        const body: Record<string, string> = { email, password }
        if (adminToken) body['admin_token'] = adminToken
        const res = await fetch('/api/auth/register', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        })
        if (!res.ok) {
          const data = await res.json().catch(() => ({})) as Record<string, unknown>
          const errObj = data?.['error'] as Record<string, unknown> | undefined
          const msg = errObj?.['message']
          throw new Error(typeof msg === 'string' ? msg : 'Registration failed')
        }
        const data = await res.json() as { token: string; user_id: string; is_admin: boolean }
        set({ token: data.token, userId: data.user_id, isAdmin: data.is_admin })
      },
      refreshProfile: async () => {
        const token = useAuthStore.getState().token
        if (!token) return
        const res = await fetch('/api/user/profile', {
          headers: { Authorization: `Bearer ${token}` },
        })
        if (!res.ok) return
        const data = await res.json() as { display_name: string | null }
        set({ displayName: data.display_name ?? null })
      },
      logout: () => {
        wsManager.disconnect()
        set({ token: null, userId: null, isAdmin: false, displayName: null })
      },
    }),
    {
      name: 'plexus-auth',
      partialize: (s) => ({ token: s.token, userId: s.userId, isAdmin: s.isAdmin, displayName: s.displayName }),
    },
  ),
)
