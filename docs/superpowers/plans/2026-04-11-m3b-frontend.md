# M3b — plexus-frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `plexus-frontend` React 19 SPA (Chat, Settings, Admin pages) in the "Cyberpunk Refined" visual style, served by `plexus-gateway` as static files.

**Architecture:** Single-page app built with Vite 8. In dev, Vite proxies `/api` and `/ws` to the gateway at port 9090. In prod, `npm run build` outputs `dist/`, which the gateway serves. Zustand stores manage auth, chat, and device state. A singleton WS manager handles reconnect, ping/pong, and message dispatch. Browser owns session IDs (`gateway:{user_id}:{uuid}`); sessions are created server-side on first message.

**Tech Stack:** React 19, TypeScript 5.9, Vite 8, Tailwind CSS 4 (`@tailwindcss/vite`), Zustand 5, react-router-dom 7, react-markdown, remark-gfm, react-syntax-highlighter, lucide-react

---

## File Map

```
plexus-frontend/
├── package.json
├── vite.config.ts          — proxy /api + /ws to localhost:9090 in dev
├── tsconfig.json
├── tsconfig.node.json
├── index.html
└── src/
    ├── main.tsx             — createRoot + StrictMode
    ├── App.tsx              — BrowserRouter, route guards (RequireAuth, RequireAdmin)
    ├── styles/
    │   └── globals.css      — @import "tailwindcss" + :root color vars + body base
    ├── lib/
    │   ├── types.ts         — shared TypeScript interfaces (Session, ChatMessage, Device…)
    │   ├── api.ts           — fetch wrapper: JWT injection, 401→logout, error shaping
    │   └── ws.ts            — WsManager singleton: connect, reconnect, ping/pong, listeners
    ├── store/
    │   ├── auth.ts          — Zustand: token, userId, isAdmin, login(), logout()
    │   ├── chat.ts          — Zustand: sessions, messages, progress, WS dispatch, loadMessages
    │   └── devices.ts       — Zustand: device list, 5s poll
    ├── pages/
    │   ├── Login.tsx        — email/password form → POST /api/auth/login
    │   ├── Chat.tsx         — URL-driven session, sidebar + message list + input
    │   ├── Settings.tsx     — tabs: Profile / Devices / Channels / Skills / Cron
    │   └── Admin.tsx        — tabs: LLM / Default Soul / Rate Limit / Server MCP
    └── components/
        ├── Sidebar.tsx           — slim collapsible session list (140–200px / 48px icon strip)
        ├── DeviceStatusBar.tsx   — top bar: session name + status dots (server + devices)
        ├── ChatInput.tsx         — auto-growing textarea, responsive min/max width
        ├── ProgressHint.tsx      — spinner + ephemeral tool hint text
        ├── MarkdownContent.tsx   — react-markdown + remark-gfm + syntax highlighter
        ├── Message.tsx           — single bubble: user (right, green tint) / agent (left, card)
        └── MessageList.tsx       — scrollable list, auto-scroll to bottom on new messages
```

---

## Task 1: Scaffold — package.json, Vite config, TypeScript config, index.html

**Files:**
- Create: `plexus-frontend/package.json`
- Create: `plexus-frontend/vite.config.ts`
- Create: `plexus-frontend/tsconfig.json`
- Create: `plexus-frontend/tsconfig.node.json`
- Create: `plexus-frontend/index.html`

- [ ] **Step 1: Create package.json**

```json
{
  "name": "plexus-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "lucide-react": "^0.525.0",
    "react": "^19.1.0",
    "react-dom": "^19.1.0",
    "react-markdown": "^10.1.0",
    "react-router-dom": "^7.5.1",
    "react-syntax-highlighter": "^15.6.1",
    "remark-gfm": "^4.0.1",
    "zustand": "^5.0.4"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.1.3",
    "@types/react": "^19.1.1",
    "@types/react-dom": "^19.1.2",
    "@types/react-syntax-highlighter": "^15.5.13",
    "@vitejs/plugin-react": "^4.4.1",
    "tailwindcss": "^4.1.3",
    "typescript": "^5.8.3",
    "vite": "^6.3.2"
  }
}
```

- [ ] **Step 2: Create vite.config.ts**

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:9090',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://localhost:9090',
        ws: true,
        changeOrigin: true,
      },
    },
  },
})
```

- [ ] **Step 3: Create tsconfig.json**

```json
{
  "files": [],
  "references": [
    { "path": "./tsconfig.node.json" },
    { "path": "./tsconfig.app.json" }
  ]
}
```

Create `tsconfig.app.json`:

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"]
}
```

Create `tsconfig.node.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 4: Create index.html**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Plexus</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 5: Install dependencies**

```bash
cd plexus-frontend && npm install
```

Expected: `node_modules/` created, no errors.

- [ ] **Step 6: Commit**

```bash
cd plexus-frontend && git add package.json vite.config.ts tsconfig.json tsconfig.app.json tsconfig.node.json index.html
git commit -m "feat(frontend): scaffold Vite + React 19 + Tailwind 4 project"
```

---

## Task 2: Global styles + entry point

**Files:**
- Create: `plexus-frontend/src/styles/globals.css`
- Create: `plexus-frontend/src/main.tsx`

- [ ] **Step 1: Create globals.css**

```css
@import "tailwindcss";

:root {
  --accent: #39ff14;
  --bg: #0d1117;
  --sidebar: #0a0f18;
  --card: #161b22;
  --border: #1a2332;
  --muted: #8b949e;
  --text: #e6edf3;
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font-family: ui-monospace, 'SFMono-Regular', 'Cascadia Code', 'Fira Code', monospace;
  font-size: 14px;
  line-height: 1.6;
}

::-webkit-scrollbar {
  width: 4px;
}
::-webkit-scrollbar-track {
  background: var(--bg);
}
::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 2px;
}
::-webkit-scrollbar-thumb:hover {
  background: var(--muted);
}
```

- [ ] **Step 2: Create src/main.tsx**

```tsx
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles/globals.css'
import App from './App'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
```

- [ ] **Step 3: Create a placeholder App.tsx to verify dev server starts**

```tsx
// src/App.tsx
export default function App() {
  return (
    <div style={{ color: 'var(--accent)', padding: 32 }}>
      Plexus — dev server OK
    </div>
  )
}
```

- [ ] **Step 4: Run dev server and verify**

```bash
cd plexus-frontend && npm run dev
```

Open `http://localhost:5173` — expect green text "Plexus — dev server OK" on dark background.

- [ ] **Step 5: Commit**

```bash
git add src/styles/globals.css src/main.tsx src/App.tsx
git commit -m "feat(frontend): global CSS theme + entry point"
```

---

## Task 3: Types

**Files:**
- Create: `plexus-frontend/src/lib/types.ts`

- [ ] **Step 1: Create src/lib/types.ts**

```ts
// User account info
export interface User {
  user_id: string
  email: string
  is_admin: boolean
  created_at: string
}

// Session (conversation thread)
export interface Session {
  session_id: string
  user_id: string
  channel: string
  created_at: string
  updated_at: string
}

// Single chat message (user or assistant)
export interface ChatMessage {
  id: string          // client UUID (optimistic) or server message_id
  session_id: string
  role: 'user' | 'assistant'
  content: string
  media?: string[]
  created_at: string
}

// Server-side API message shape (from GET /api/sessions/{id}/messages)
export interface ApiMessage {
  message_id: string
  session_id: string
  role: 'user' | 'assistant'
  content: string
  created_at: string
}

// Connected client device
export interface Device {
  device_name: string
  online: boolean
  tool_count: number
  last_seen: string
}

// Device auth token
export interface DeviceToken {
  token: string
  device_name: string
  created_at: string
  last_used: string | null
}

// Per-device filesystem policy
export interface DevicePolicy {
  fs_policy: { mode: 'sandbox' | 'unrestricted' }
  workspace_path: string
  shell_timeout: number
  ssrf_whitelist: string[]
}

// MCP server config entry (used by both server and client MCP)
export interface McpServerEntry {
  name: string
  command: string
  args: string[]
  env?: Record<string, string>
  url?: string
  enabled: boolean
  tool_timeout?: number
}

// Discord channel config
export interface DiscordConfig {
  user_id: string
  bot_user_id: string
  enabled: boolean
  partner_discord_id: string
  allowed_users: string[]
}

// Telegram channel config
export interface TelegramConfig {
  partner_telegram_id: string
  allowed_users: string[]
  group_policy: 'mention' | 'all'
}

// LLM provider config (admin only)
export interface LlmConfig {
  api_base: string
  model: string
  api_key: string
  context_window: number
}

// Cron job
export interface CronJob {
  job_id: string
  user_id: string
  message: string
  name: string | null
  enabled: boolean
  cron_expr: string | null
  every_seconds: number | null
  at: string | null
  timezone: string | null
  channel: string | null
  created_at: string
}

// User skill
export interface Skill {
  name: string
  description: string
  always_on: boolean
  created_at: string
}

// Rate limit config (admin only)
export interface RateLimit {
  rate_limit_per_min: number
}

// Default soul config (admin only)
export interface DefaultSoul {
  soul: string
}
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors (types file has no imports to resolve).

- [ ] **Step 3: Commit**

```bash
git add src/lib/types.ts
git commit -m "feat(frontend): TypeScript types for all API shapes"
```

---

## Task 4: API client

**Files:**
- Create: `plexus-frontend/src/lib/api.ts`

- [ ] **Step 1: Create src/lib/api.ts**

```ts
// Thin fetch wrapper. Injects JWT, shapes errors, triggers logout on 401.
// Import useAuthStore lazily to avoid circular deps at module init.

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  // Lazy import avoids circular dependency at module load time
  const { useAuthStore } = await import('../store/auth')
  const token = useAuthStore.getState().token

  const headers: Record<string, string> = {}
  if (token) headers['Authorization'] = `Bearer ${token}`
  if (body !== undefined) headers['Content-Type'] = 'application/json'

  const res = await fetch(path, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })

  if (res.status === 401) {
    useAuthStore.getState().logout()
    throw new Error('Session expired — please log in again')
  }

  if (!res.ok) {
    const json = await res.json().catch(() => ({})) as Record<string, unknown>
    const msg = (json?.error as Record<string, unknown> | undefined)?.message
    throw new Error(typeof msg === 'string' ? msg : `Request failed: HTTP ${res.status}`)
  }

  // Some endpoints return 204 No Content
  if (res.status === 204) return undefined as T

  return res.json() as Promise<T>
}

export const api = {
  get:    <T>(path: string)                 => request<T>('GET',    path),
  post:   <T>(path: string, body: unknown)  => request<T>('POST',   path, body),
  put:    <T>(path: string, body: unknown)  => request<T>('PUT',    path, body),
  patch:  <T>(path: string, body: unknown)  => request<T>('PATCH',  path, body),
  delete: <T>(path: string)                 => request<T>('DELETE', path),
}
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/lib/api.ts
git commit -m "feat(frontend): API fetch wrapper with JWT injection and 401 logout"
```

---

## Task 5: WebSocket manager

**Files:**
- Create: `plexus-frontend/src/lib/ws.ts`

- [ ] **Step 1: Create src/lib/ws.ts**

```ts
export type WsStatus = 'connecting' | 'open' | 'closed'
type MessageHandler = (data: Record<string, unknown>) => void
type StatusHandler = (status: WsStatus) => void

// Reconnect backoff in ms — capped at 30s
const BACKOFF = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000]

class WsManager {
  private ws: WebSocket | null = null
  private token: string | null = null
  private attempt = 0
  private disposed = false
  private status: WsStatus = 'closed'
  private retryTimer: ReturnType<typeof setTimeout> | null = null
  private messageHandlers = new Set<MessageHandler>()
  private statusHandlers = new Set<StatusHandler>()

  /** Idempotent — no-op if already connected with the same token. */
  connect(token: string) {
    if (
      this.token === token &&
      this.ws !== null &&
      (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN)
    ) return
    this.token = token
    this.disposed = false
    this.attempt = 0
    this.openSocket()
  }

  private openSocket() {
    if (this.disposed || !this.token) return
    this.ws?.close()

    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const url = `${proto}//${window.location.host}/ws/chat?token=${encodeURIComponent(this.token)}`

    this.setStatus('connecting')
    const ws = new WebSocket(url)
    this.ws = ws

    ws.onopen = () => {
      this.attempt = 0
      this.setStatus('open')
    }

    ws.onmessage = (e: MessageEvent) => {
      let data: Record<string, unknown>
      try {
        data = JSON.parse(e.data as string) as Record<string, unknown>
      } catch {
        return
      }
      if (data['type'] === 'ping') {
        ws.send(JSON.stringify({ type: 'pong' }))
        return
      }
      this.messageHandlers.forEach(h => h(data))
    }

    ws.onclose = () => {
      if (this.disposed) return
      this.setStatus('closed')
      this.scheduleReconnect()
    }

    // onerror is always followed by onclose — no separate handling needed
    ws.onerror = () => {}
  }

  private scheduleReconnect() {
    const base = BACKOFF[Math.min(this.attempt, BACKOFF.length - 1)]
    const delay = base * (0.75 + Math.random() * 0.5)
    this.attempt++
    this.retryTimer = setTimeout(() => this.openSocket(), delay)
  }

  send(data: unknown) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(data))
    }
  }

  /** Register a message handler. Returns an unsubscribe function. */
  onMessage(handler: MessageHandler): () => void {
    this.messageHandlers.add(handler)
    return () => this.messageHandlers.delete(handler)
  }

  /** Register a status handler. Returns an unsubscribe function. */
  onStatus(handler: StatusHandler): () => void {
    this.statusHandlers.add(handler)
    return () => this.statusHandlers.delete(handler)
  }

  getStatus(): WsStatus {
    return this.status
  }

  disconnect() {
    this.disposed = true
    if (this.retryTimer !== null) clearTimeout(this.retryTimer)
    this.ws?.close()
    this.ws = null
    this.token = null
    this.attempt = 0
    this.setStatus('closed')
  }

  private setStatus(s: WsStatus) {
    this.status = s
    this.statusHandlers.forEach(h => h(s))
  }
}

export const wsManager = new WsManager()
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/lib/ws.ts
git commit -m "feat(frontend): WS manager singleton with auto-reconnect and ping/pong"
```

---

## Task 6: Auth store

**Files:**
- Create: `plexus-frontend/src/store/auth.ts`

- [ ] **Step 1: Create src/store/auth.ts**

```ts
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
          const msg = (body?.error as Record<string, unknown> | undefined)?.message
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
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/store/auth.ts
git commit -m "feat(frontend): auth store with persisted JWT and login/logout"
```

---

## Task 7: Chat store

**Files:**
- Create: `plexus-frontend/src/store/chat.ts`

- [ ] **Step 1: Create src/store/chat.ts**

```ts
import { create } from 'zustand'
import { api } from '../lib/api'
import { wsManager, WsStatus } from '../lib/ws'
import type { Session, ChatMessage, ApiMessage } from '../lib/types'

interface ChatState {
  sessions: Session[]
  currentSessionId: string | null
  messagesBySession: Record<string, ChatMessage[]>
  restLoadedSessions: Set<string>
  progressBySession: Record<string, string | null>
  wsStatus: WsStatus

  init: () => void
  loadSessions: () => Promise<void>
  loadMessages: (sessionId: string) => Promise<void>
  setCurrentSession: (sessionId: string | null) => void
  sendMessage: (sessionId: string, content: string, media?: string[]) => void
  handleIncomingMessage: (sessionId: string, content: string, media?: string[] | undefined) => void
  setProgressHint: (sessionId: string, hint: string) => void
  clearProgress: (sessionId: string) => void
  handleError: (reason: string) => void
}

// Module-level init guard (survives React StrictMode double-invocations)
let initialized = false
let unsubMsg: (() => void) | null = null
let unsubStatus: (() => void) | null = null

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  messagesBySession: {},
  restLoadedSessions: new Set(),
  progressBySession: {},
  wsStatus: 'closed',

  init: () => {
    if (initialized) return
    initialized = true

    unsubMsg = wsManager.onMessage((data) => {
      const store = get()
      const sessionId = data['session_id'] as string | undefined
      if (!sessionId) return

      if (data['type'] === 'message') {
        store.handleIncomingMessage(sessionId, data['content'] as string, data['media'] as string[] | undefined)
      } else if (data['type'] === 'progress') {
        store.setProgressHint(sessionId, data['content'] as string)
      } else if (data['type'] === 'error') {
        store.handleError(data['reason'] as string ?? 'Unknown error')
      }
    })

    unsubStatus = wsManager.onStatus((status) => {
      set({ wsStatus: status })
    })

    set({ wsStatus: wsManager.getStatus() })
  },

  loadSessions: async () => {
    const sessions = await api.get<Session[]>('/api/sessions')
    set({ sessions })
  },

  loadMessages: async (sessionId) => {
    // Guard: only load once per session per page load
    if (get().restLoadedSessions.has(sessionId)) return

    const apiMsgs = await api.get<ApiMessage[]>(
      `/api/sessions/${sessionId}/messages?limit=200`,
    )

    const msgs: ChatMessage[] = apiMsgs.map(m => ({
      id: m.message_id,
      session_id: m.session_id,
      role: m.role,
      content: m.content,
      created_at: m.created_at,
    }))

    set(s => {
      const existing = s.messagesBySession[sessionId] ?? []
      // Prepend REST history; preserve any WS messages that arrived during fetch
      const merged = [...msgs, ...existing].sort(
        (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
      )
      const loaded = new Set(s.restLoadedSessions)
      loaded.add(sessionId)
      return {
        messagesBySession: { ...s.messagesBySession, [sessionId]: merged },
        restLoadedSessions: loaded,
      }
    })
  },

  setCurrentSession: (sessionId) => {
    set(s => ({
      currentSessionId: sessionId,
      // Clear progress for previous session
      progressBySession: sessionId
        ? { ...s.progressBySession, [s.currentSessionId ?? '']: null }
        : s.progressBySession,
    }))
  },

  sendMessage: (sessionId, content, media) => {
    // Optimistic local echo
    const localMsg: ChatMessage = {
      id: crypto.randomUUID(),
      session_id: sessionId,
      role: 'user',
      content,
      media,
      created_at: new Date().toISOString(),
    }
    set(s => ({
      messagesBySession: {
        ...s.messagesBySession,
        [sessionId]: [...(s.messagesBySession[sessionId] ?? []), localMsg],
      },
    }))

    // Send over WS
    wsManager.send({
      type: 'message',
      session_id: sessionId,
      content,
      ...(media && media.length > 0 ? { media } : {}),
    })
  },

  handleIncomingMessage: (sessionId, content, media) => {
    const msg: ChatMessage = {
      id: crypto.randomUUID(),
      session_id: sessionId,
      role: 'assistant',
      content,
      media,
      created_at: new Date().toISOString(),
    }
    set(s => ({
      messagesBySession: {
        ...s.messagesBySession,
        [sessionId]: [...(s.messagesBySession[sessionId] ?? []), msg],
      },
      // Clear progress when a final message arrives
      progressBySession: { ...s.progressBySession, [sessionId]: null },
    }))
  },

  setProgressHint: (sessionId, hint) => {
    set(s => ({
      progressBySession: { ...s.progressBySession, [sessionId]: hint },
    }))
  },

  clearProgress: (sessionId) => {
    set(s => ({
      progressBySession: { ...s.progressBySession, [sessionId]: null },
    }))
  },

  handleError: (reason) => {
    // Show as a browser console warn; the WS status indicator covers visual feedback
    console.warn('[plexus] WS error:', reason)
  },
}))

// Cleanup export for tests / logout
export function resetChatStore() {
  initialized = false
  unsubMsg?.()
  unsubStatus?.()
  unsubMsg = null
  unsubStatus = null
  useChatStore.setState({
    sessions: [],
    currentSessionId: null,
    messagesBySession: {},
    restLoadedSessions: new Set(),
    progressBySession: {},
    wsStatus: 'closed',
  })
}
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/store/chat.ts
git commit -m "feat(frontend): chat store — sessions, messages, WS dispatch, progress hints"
```

---

## Task 8: Devices store

**Files:**
- Create: `plexus-frontend/src/store/devices.ts`

- [ ] **Step 1: Create src/store/devices.ts**

```ts
import { create } from 'zustand'
import { api } from '../lib/api'
import type { Device } from '../lib/types'

interface DevicesState {
  devices: Device[]
  loading: boolean
  fetchDevices: () => Promise<void>
  startPolling: () => () => void   // returns a cleanup fn (call on unmount)
}

export const useDevicesStore = create<DevicesState>((set, get) => ({
  devices: [],
  loading: false,

  fetchDevices: async () => {
    set({ loading: true })
    try {
      const devices = await api.get<Device[]>('/api/devices')
      set({ devices })
    } catch {
      // silently ignore — UI shows stale state
    } finally {
      set({ loading: false })
    }
  },

  startPolling: () => {
    const { fetchDevices } = get()
    void fetchDevices()
    const id = setInterval(() => void fetchDevices(), 5_000)
    return () => clearInterval(id)
  },
}))
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/store/devices.ts
git commit -m "feat(frontend): devices store with 5s polling"
```

---

## Task 9: Router + App.tsx

**Files:**
- Modify: `plexus-frontend/src/App.tsx`

- [ ] **Step 1: Rewrite App.tsx with routing and guards**

```tsx
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { useAuthStore } from './store/auth'
import Login from './pages/Login'
import Chat from './pages/Chat'
import Settings from './pages/Settings'
import Admin from './pages/Admin'

function RequireAuth({ children }: { children: React.ReactNode }) {
  const token = useAuthStore(s => s.token)
  if (!token) return <Navigate to="/login" replace />
  return <>{children}</>
}

function RequireAdmin({ children }: { children: React.ReactNode }) {
  const { token, isAdmin } = useAuthStore(s => ({ token: s.token, isAdmin: s.isAdmin }))
  if (!token) return <Navigate to="/login" replace />
  if (!isAdmin) return <Navigate to="/chat" replace />
  return <>{children}</>
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/" element={<Navigate to="/chat" replace />} />
        <Route path="/chat" element={<RequireAuth><Chat /></RequireAuth>} />
        <Route path="/chat/:sessionId" element={<RequireAuth><Chat /></RequireAuth>} />
        <Route path="/settings" element={<RequireAuth><Settings /></RequireAuth>} />
        <Route path="/admin" element={<RequireAdmin><Admin /></RequireAdmin>} />
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </BrowserRouter>
  )
}
```

- [ ] **Step 2: Create stub pages so type-check passes (Login, Chat, Settings, Admin)**

Create `src/pages/Login.tsx`:
```tsx
export default function Login() { return <div>Login</div> }
```

Create `src/pages/Chat.tsx`:
```tsx
export default function Chat() { return <div>Chat</div> }
```

Create `src/pages/Settings.tsx`:
```tsx
export default function Settings() { return <div>Settings</div> }
```

Create `src/pages/Admin.tsx`:
```tsx
export default function Admin() { return <div>Admin</div> }
```

- [ ] **Step 3: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 4: Verify in browser**

```bash
npm run dev
```

Open `http://localhost:5173` — expect redirect to `http://localhost:5173/login`.

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/pages/Login.tsx src/pages/Chat.tsx src/pages/Settings.tsx src/pages/Admin.tsx
git commit -m "feat(frontend): router with RequireAuth and RequireAdmin guards"
```

---

## Task 10: Login page

**Files:**
- Modify: `plexus-frontend/src/pages/Login.tsx`

- [ ] **Step 1: Write Login.tsx**

```tsx
import { useState, FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuthStore } from '../store/auth'

export default function Login() {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const login = useAuthStore(s => s.login)
  const navigate = useNavigate()

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await login(email, password)
      navigate('/chat', { replace: true })
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed')
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
        <div className="mb-8 text-center">
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
              style={{
                background: 'var(--bg)',
                color: 'var(--text)',
                borderColor: 'var(--border)',
              }}
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
              style={{
                background: 'var(--bg)',
                color: 'var(--text)',
                borderColor: 'var(--border)',
              }}
            />
          </div>

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
            {loading ? 'Signing in…' : 'Sign In'}
          </button>
        </form>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Verify in browser**

Start dev server (`npm run dev`). Open `http://localhost:5173/login`. Verify:
- Dark background, green "PLEXUS" title with glow
- Email + password fields, Sign In button
- Submit calls `POST /api/auth/login` (proxy to gateway at 9090)
- On success: redirect to `/chat`
- On failure: red error message appears

- [ ] **Step 4: Commit**

```bash
git add src/pages/Login.tsx
git commit -m "feat(frontend): login page with Cyberpunk Refined styling"
```

---

## Task 11: Core layout components

**Files:**
- Create: `plexus-frontend/src/components/Sidebar.tsx`
- Create: `plexus-frontend/src/components/DeviceStatusBar.tsx`
- Create: `plexus-frontend/src/components/ChatInput.tsx`
- Create: `plexus-frontend/src/components/ProgressHint.tsx`

- [ ] **Step 1: Create Sidebar.tsx**

```tsx
import { useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { MessageSquare, Plus, ChevronLeft, ChevronRight, Settings, Shield } from 'lucide-react'
import { useChatStore } from '../store/chat'
import { useAuthStore } from '../store/auth'

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const { sessionId: activeId } = useParams<{ sessionId: string }>()
  const sessions = useChatStore(s => s.sessions)
  const isAdmin = useAuthStore(s => s.isAdmin)
  const navigate = useNavigate()

  function newSession() {
    const userId = useAuthStore.getState().userId ?? 'unknown'
    const uuid = crypto.randomUUID()
    navigate(`/chat/gateway:${userId}:${uuid}`)
  }

  return (
    <aside
      className="flex flex-col h-full border-r shrink-0 transition-all duration-200"
      style={{
        width: collapsed ? 48 : 200,
        background: 'var(--sidebar)',
        borderColor: 'var(--border)',
      }}
    >
      {/* Toggle button */}
      <div className="flex items-center justify-between px-2 py-3 border-b" style={{ borderColor: 'var(--border)' }}>
        {!collapsed && (
          <span className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--accent)' }}>
            Plexus
          </span>
        )}
        <button
          onClick={() => setCollapsed(c => !c)}
          className="p-1 rounded hover:bg-[#1a2332] transition-colors ml-auto"
          style={{ color: 'var(--muted)' }}
        >
          {collapsed ? <ChevronRight size={14} /> : <ChevronLeft size={14} />}
        </button>
      </div>

      {/* New chat */}
      <div className="px-2 py-2 border-b" style={{ borderColor: 'var(--border)' }}>
        <button
          onClick={newSession}
          className="flex items-center gap-2 w-full rounded-lg px-2 py-1.5 text-xs transition-colors hover:bg-[#1a2332]"
          style={{ color: 'var(--accent)' }}
          title="New chat"
        >
          <Plus size={14} />
          {!collapsed && <span>New chat</span>}
        </button>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto py-1">
        {sessions.map(session => {
          const isActive = session.session_id === activeId
          return (
            <button
              key={session.session_id}
              onClick={() => navigate(`/chat/${session.session_id}`)}
              className="flex items-center gap-2 w-full px-3 py-2 text-xs text-left transition-colors hover:bg-[#1a2332] rounded"
              style={{
                color: isActive ? 'var(--accent)' : 'var(--text)',
                background: isActive ? 'rgba(57,255,20,0.06)' : 'transparent',
              }}
              title={session.session_id}
            >
              <MessageSquare size={12} style={{ flexShrink: 0, color: isActive ? 'var(--accent)' : 'var(--muted)' }} />
              {!collapsed && (
                <span className="truncate">
                  {session.session_id.split(':')[2]?.slice(0, 8) ?? session.session_id}
                </span>
              )}
            </button>
          )
        })}
      </div>

      {/* Bottom nav */}
      <div className="border-t p-2 flex flex-col gap-1" style={{ borderColor: 'var(--border)' }}>
        <button
          onClick={() => navigate('/settings')}
          className="flex items-center gap-2 w-full rounded-lg px-2 py-1.5 text-xs transition-colors hover:bg-[#1a2332]"
          style={{ color: 'var(--muted)' }}
          title="Settings"
        >
          <Settings size={14} />
          {!collapsed && <span>Settings</span>}
        </button>
        {isAdmin && (
          <button
            onClick={() => navigate('/admin')}
            className="flex items-center gap-2 w-full rounded-lg px-2 py-1.5 text-xs transition-colors hover:bg-[#1a2332]"
            style={{ color: 'var(--muted)' }}
            title="Admin"
          >
            <Shield size={14} />
            {!collapsed && <span>Admin</span>}
          </button>
        )}
      </div>
    </aside>
  )
}
```

- [ ] **Step 2: Create DeviceStatusBar.tsx**

```tsx
import { useChatStore } from '../store/chat'
import { useDevicesStore } from '../store/devices'

interface Props {
  sessionId: string
}

export default function DeviceStatusBar({ sessionId }: Props) {
  const wsStatus = useChatStore(s => s.wsStatus)
  const devices = useDevicesStore(s => s.devices)

  const wsColor = wsStatus === 'open' ? '#39ff14' : wsStatus === 'connecting' ? '#facc15' : '#ef4444'
  const wsGlow = wsStatus === 'open' ? '0 0 6px #39ff14' : 'none'

  const shortId = sessionId.split(':')[2]?.slice(0, 8) ?? sessionId

  return (
    <div
      className="flex items-center gap-3 px-4 py-2 border-b text-xs"
      style={{ background: 'var(--sidebar)', borderColor: 'var(--border)', color: 'var(--muted)' }}
    >
      <span style={{ color: 'var(--text)' }} className="font-mono">
        {shortId}
      </span>

      <div className="flex items-center gap-1 ml-auto">
        {/* Gateway WS status */}
        <span style={{ color: 'var(--muted)' }}>gateway</span>
        <span
          className="rounded-full"
          style={{ width: 7, height: 7, background: wsColor, boxShadow: wsGlow, display: 'inline-block' }}
        />
      </div>

      {/* Per-device dots */}
      {devices.map(d => (
        <div key={d.device_name} className="flex items-center gap-1">
          <span style={{ color: 'var(--muted)' }}>{d.device_name}</span>
          <span
            className="rounded-full"
            style={{
              width: 7,
              height: 7,
              display: 'inline-block',
              background: d.online ? '#39ff14' : '#ef4444',
              boxShadow: d.online ? '0 0 6px #39ff14' : 'none',
            }}
          />
        </div>
      ))}
    </div>
  )
}
```

- [ ] **Step 3: Create ChatInput.tsx**

```tsx
import { useState, useRef, KeyboardEvent } from 'react'
import { Send } from 'lucide-react'

interface Props {
  onSend: (content: string) => void
  disabled?: boolean
}

export default function ChatInput({ onSend, disabled }: Props) {
  const [value, setValue] = useState('')
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  function submit() {
    const trimmed = value.trim()
    if (!trimmed || disabled) return
    onSend(trimmed)
    setValue('')
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto'
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
    }
  }

  function handleInput() {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 200) + 'px'
  }

  return (
    <div
      className="flex items-end gap-2 rounded-xl border p-3"
      style={{
        background: 'var(--card)',
        borderColor: 'var(--border)',
        width: 'min(90vw, 720px)',
        minWidth: 'min(90vw, 420px)',
      }}
    >
      <textarea
        ref={textareaRef}
        value={value}
        onChange={e => setValue(e.target.value)}
        onInput={handleInput}
        onKeyDown={handleKeyDown}
        disabled={disabled}
        placeholder="Message Plexus… (Enter to send, Shift+Enter for newline)"
        rows={1}
        className="flex-1 resize-none outline-none text-sm bg-transparent"
        style={{ color: 'var(--text)', maxHeight: 200 }}
      />
      <button
        onClick={submit}
        disabled={disabled || !value.trim()}
        className="p-1.5 rounded-lg transition-all disabled:opacity-30"
        style={{ color: 'var(--accent)' }}
        title="Send"
      >
        <Send size={16} />
      </button>
    </div>
  )
}
```

- [ ] **Step 4: Create ProgressHint.tsx**

```tsx
interface Props {
  hint: string
}

export default function ProgressHint({ hint }: Props) {
  return (
    <div className="flex items-center gap-2 px-2 py-1 text-xs" style={{ color: 'var(--muted)' }}>
      {/* Spinning dot */}
      <span
        className="rounded-full animate-spin"
        style={{
          width: 8,
          height: 8,
          display: 'inline-block',
          border: '2px solid transparent',
          borderTopColor: 'var(--accent)',
          flexShrink: 0,
        }}
      />
      <span className="truncate">{hint}</span>
    </div>
  )
}
```

- [ ] **Step 5: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/components/Sidebar.tsx src/components/DeviceStatusBar.tsx src/components/ChatInput.tsx src/components/ProgressHint.tsx
git commit -m "feat(frontend): Sidebar, DeviceStatusBar, ChatInput, ProgressHint components"
```

---

## Task 12: Message rendering components

**Files:**
- Create: `plexus-frontend/src/components/MarkdownContent.tsx`
- Create: `plexus-frontend/src/components/Message.tsx`
- Create: `plexus-frontend/src/components/MessageList.tsx`

- [ ] **Step 1: Create MarkdownContent.tsx**

```tsx
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism'
import type { Components } from 'react-markdown'

interface Props {
  content: string
}

const components: Components = {
  code({ className, children, ...props }) {
    const match = /language-(\w+)/.exec(className ?? '')
    const isBlock = Boolean(match)
    if (isBlock) {
      return (
        <SyntaxHighlighter
          style={vscDarkPlus}
          language={match![1]}
          PreTag="div"
          customStyle={{
            borderRadius: 8,
            fontSize: 12,
            marginTop: 8,
            marginBottom: 8,
            background: '#0d1117',
            border: '1px solid #1a2332',
          }}
        >
          {String(children).replace(/\n$/, '')}
        </SyntaxHighlighter>
      )
    }
    return (
      <code
        className={className}
        style={{ color: '#39ff14', background: '#161b22', padding: '1px 5px', borderRadius: 4, fontSize: 12 }}
        {...props}
      >
        {children}
      </code>
    )
  },
  a({ href, children }) {
    return (
      <a href={href} target="_blank" rel="noreferrer" style={{ color: '#39ff14', textDecoration: 'underline' }}>
        {children}
      </a>
    )
  },
}

export default function MarkdownContent({ content }: Props) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {content}
    </ReactMarkdown>
  )
}
```

- [ ] **Step 2: Create Message.tsx**

```tsx
import type { ChatMessage } from '../lib/types'
import MarkdownContent from './MarkdownContent'

interface Props {
  message: ChatMessage
}

export default function Message({ message }: Props) {
  const isUser = message.role === 'user'

  if (isUser) {
    return (
      <div className="flex justify-end mb-3">
        <div
          className="max-w-[70%] px-4 py-2 text-sm"
          style={{
            background: 'rgba(57,255,20,0.08)',
            color: 'var(--text)',
            borderRadius: '12px 12px 2px 12px',
            border: '1px solid rgba(57,255,20,0.15)',
          }}
        >
          {message.content}
        </div>
      </div>
    )
  }

  return (
    <div className="flex justify-start mb-3">
      <div
        className="max-w-[80%] px-4 py-3 text-sm"
        style={{
          background: 'var(--card)',
          color: 'var(--text)',
          borderRadius: '2px 12px 12px 12px',
          borderLeft: '3px solid var(--accent)',
        }}
      >
        <MarkdownContent content={message.content} />
        {message.media && message.media.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1">
            {message.media.map((url, i) => (
              <a
                key={i}
                href={url}
                target="_blank"
                rel="noreferrer"
                className="text-xs underline"
                style={{ color: 'var(--accent)' }}
              >
                Attachment {i + 1}
              </a>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
```

- [ ] **Step 3: Create MessageList.tsx**

```tsx
import { useEffect, useRef } from 'react'
import type { ChatMessage } from '../lib/types'
import Message from './Message'
import ProgressHint from './ProgressHint'

interface Props {
  messages: ChatMessage[]
  progressHint: string | null
}

export default function MessageList({ messages, progressHint }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to bottom when messages or progress changes
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages.length, progressHint])

  return (
    <div className="flex-1 overflow-y-auto px-4 py-4">
      {messages.map(msg => (
        <Message key={msg.id} message={msg} />
      ))}
      {progressHint && <ProgressHint hint={progressHint} />}
      <div ref={bottomRef} />
    </div>
  )
}
```

- [ ] **Step 4: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/components/MarkdownContent.tsx src/components/Message.tsx src/components/MessageList.tsx
git commit -m "feat(frontend): MarkdownContent, Message bubble, MessageList with auto-scroll"
```

---

## Task 13: Chat page

**Files:**
- Modify: `plexus-frontend/src/pages/Chat.tsx`

- [ ] **Step 1: Write Chat.tsx**

```tsx
import { useEffect } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { useAuthStore } from '../store/auth'
import { useChatStore } from '../store/chat'
import { useDevicesStore } from '../store/devices'
import { wsManager } from '../lib/ws'
import Sidebar from '../components/Sidebar'
import DeviceStatusBar from '../components/DeviceStatusBar'
import MessageList from '../components/MessageList'
import ChatInput from '../components/ChatInput'

export default function Chat() {
  const { sessionId } = useParams<{ sessionId: string }>()
  const navigate = useNavigate()
  const { token, userId } = useAuthStore(s => ({ token: s.token, userId: s.userId }))
  const { init, loadSessions, loadMessages, sendMessage, setCurrentSession, messagesBySession, progressBySession, wsStatus } = useChatStore()
  const startPolling = useDevicesStore(s => s.startPolling)

  // Generate session ID if URL has none
  useEffect(() => {
    if (!sessionId && userId) {
      navigate(`/chat/gateway:${userId}:${crypto.randomUUID()}`, { replace: true })
    }
  }, [sessionId, userId, navigate])

  // Init WS and chat store once
  useEffect(() => {
    if (!token) return
    init()
    wsManager.connect(token)
    void loadSessions()
    return startPolling()
  }, [token]) // eslint-disable-line react-hooks/exhaustive-deps

  // Load messages when session changes
  useEffect(() => {
    if (!sessionId) return
    setCurrentSession(sessionId)
    void loadMessages(sessionId)
  }, [sessionId]) // eslint-disable-line react-hooks/exhaustive-deps

  const messages = sessionId ? (messagesBySession[sessionId] ?? []) : []
  const progress = sessionId ? (progressBySession[sessionId] ?? null) : null
  const isConnected = wsStatus === 'open'

  function handleSend(content: string) {
    if (!sessionId) return
    sendMessage(sessionId, content)
  }

  const hasMessages = messages.length > 0

  return (
    <div className="flex h-screen" style={{ background: 'var(--bg)' }}>
      <Sidebar />

      <div className="flex flex-col flex-1 min-w-0">
        {sessionId && <DeviceStatusBar sessionId={sessionId} />}

        {!hasMessages ? (
          /* Empty state */
          <div className="flex flex-col flex-1 items-center justify-center gap-6">
            <div className="text-center">
              <p className="text-xl font-semibold" style={{ color: 'var(--text)' }}>
                Hey, {userId?.slice(0, 8) ?? 'there'}
              </p>
              <p className="text-sm mt-1" style={{ color: 'var(--muted)' }}>
                {isConnected ? 'What can I help you with?' : 'Connecting to Plexus…'}
              </p>
            </div>
            <ChatInput onSend={handleSend} disabled={!isConnected} />
          </div>
        ) : (
          /* Active state */
          <>
            <MessageList messages={messages} progressHint={progress} />
            <div className="flex justify-center pb-4 pt-2">
              <ChatInput onSend={handleSend} disabled={!isConnected} />
            </div>
          </>
        )}
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Manual validation**

Start gateway + server, then:

```bash
cd plexus-frontend && npm run dev
```

1. Log in at `http://localhost:5173/login` with an existing account.
2. Expect redirect to `/chat` → auto-redirect to `/chat/gateway:{userId}:{uuid}`.
3. Sidebar loads session list (may be empty for new account).
4. Device status bar shows session UUID prefix + gateway status dot (green if WS connected).
5. Type "hello" and press Enter — user bubble appears immediately, agent response arrives.
6. Progress hints appear as spinning dot during agent tool calls.
7. "New chat" button in sidebar navigates to a fresh session.
8. Clicking a session in the sidebar navigates and loads its messages.

- [ ] **Step 4: Commit**

```bash
git add src/pages/Chat.tsx
git commit -m "feat(frontend): Chat page — URL-driven sessions, WS, empty/active state"
```

---

## Task 14: Settings page

**Files:**
- Modify: `plexus-frontend/src/pages/Settings.tsx`

- [ ] **Step 1: Write Settings.tsx**

```tsx
import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { api } from '../lib/api'
import type { User, Device, DeviceToken, DevicePolicy, McpServerEntry, DiscordConfig, TelegramConfig, Skill, CronJob } from '../lib/types'

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

        {/* Tab bar */}
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

// ── Profile Tab ──────────────────────────────────────────────────────────────

function ProfileTab() {
  const [profile, setProfile] = useState<User | null>(null)
  const [soul, setSoul] = useState('')
  const [memory, setMemory] = useState('')
  const [saving, setSaving] = useState(false)
  const [msg, setMsg] = useState('')

  useEffect(() => {
    void (async () => {
      const [p, s, m] = await Promise.all([
        api.get<User>('/api/user/profile'),
        api.get<{ soul: string }>('/api/user/soul'),
        api.get<{ memory: string }>('/api/user/memory'),
      ])
      setProfile(p)
      setSoul(s.soul ?? '')
      setMemory(m.memory ?? '')
    })()
  }, [])

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
    </div>
  )
}

// ── Devices Tab ──────────────────────────────────────────────────────────────

function DevicesTab() {
  const [devices, setDevices] = useState<Device[]>([])
  const [tokens, setTokens] = useState<DeviceToken[]>([])
  const [newTokenName, setNewTokenName] = useState('')
  const [createdToken, setCreatedToken] = useState('')
  const [expandedDevice, setExpandedDevice] = useState<string | null>(null)
  const [policies, setPolicies] = useState<Record<string, DevicePolicy>>({})
  const [mcpConfigs, setMcpConfigs] = useState<Record<string, string>>({})
  const [msg, setMsg] = useState('')

  useEffect(() => {
    void refresh()
  }, [])

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
      {/* Token creation */}
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

      {/* Token list */}
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

      {/* Device list */}
      <Section title="Connected Devices">
        {devices.length === 0 && <p className="text-xs" style={{ color: 'var(--muted)' }}>No devices connected.</p>}
        {devices.map(d => (
          <div key={d.device_name} className="border rounded-lg overflow-hidden mb-2" style={{ borderColor: 'var(--border)' }}>
            <button
              onClick={() => expandDevice(d.device_name)}
              className="w-full flex items-center justify-between px-4 py-3 text-sm hover:bg-[#1a2332] transition-colors"
            >
              <div className="flex items-center gap-2">
                <span style={{ width: 8, height: 8, borderRadius: '50%', background: d.online ? '#39ff14' : '#ef4444', boxShadow: d.online ? '0 0 6px #39ff14' : 'none', display: 'inline-block' }} />
                <span style={{ color: 'var(--text)' }}>{d.device_name}</span>
                <span style={{ color: 'var(--muted)' }}>({d.tool_count} tools)</span>
              </div>
              <span style={{ color: 'var(--muted)', fontSize: 10 }}>{expandedDevice === d.device_name ? '▲' : '▼'}</span>
            </button>

            {expandedDevice === d.device_name && policies[d.device_name] && (
              <div className="px-4 pb-4 flex flex-col gap-4 border-t" style={{ borderColor: 'var(--border)' }}>
                <div className="mt-3">
                  <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>Filesystem Policy</label>
                  <select
                    value={policies[d.device_name].fs_policy.mode}
                    onChange={e => setPolicies(p => ({ ...p, [d.device_name]: { ...p[d.device_name], fs_policy: { mode: e.target.value as 'sandbox' | 'unrestricted' } } }))}
                    className="mt-1 w-full rounded-lg px-3 py-2 text-sm outline-none border"
                    style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
                  >
                    <option value="sandbox">Sandbox (workspace only)</option>
                    <option value="unrestricted">Unrestricted (full access)</option>
                  </select>
                  <SaveButton onClick={() => savePolicy(d.device_name)} label="Save Policy" loading={false} />
                </div>

                <div>
                  <label className="text-xs uppercase tracking-wider" style={{ color: 'var(--muted)' }}>MCP Servers (JSON)</label>
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

// ── Channels Tab ─────────────────────────────────────────────────────────────

function ChannelsTab() {
  const [discord, setDiscord] = useState<DiscordConfig | null>(null)
  const [dcForm, setDcForm] = useState({ bot_token: '', partner_discord_id: '', allowed_users: '' })
  const [telegram, setTelegram] = useState<TelegramConfig | null>(null)
  const [tgForm, setTgForm] = useState({ bot_token: '', partner_telegram_id: '', allowed_users: '', group_policy: 'mention' as 'mention' | 'all' })
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
      {/* Discord */}
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

      {/* Telegram */}
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
        {skills.length === 0 && <p className="text-xs" style={{ color: 'var(--muted)' }}>No skills installed.</p>}
        {skills.map(s => (
          <div key={s.name} className="flex items-center justify-between py-2 border-b text-sm" style={{ borderColor: 'var(--border)' }}>
            <div>
              <span style={{ color: 'var(--text)' }}>{s.name}</span>
              {s.always_on && <span className="ml-2 text-xs px-1 rounded" style={{ background: 'rgba(57,255,20,0.1)', color: 'var(--accent)' }}>always-on</span>}
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
          placeholder="---&#10;name: my-skill&#10;description: Does something&#10;---&#10;&#10;# My Skill&#10;..."
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
        {jobs.length === 0 && <p className="text-xs" style={{ color: 'var(--muted)' }}>No cron jobs.</p>}
        {jobs.map(j => (
          <div key={j.job_id} className="flex items-start justify-between py-2 border-b text-xs" style={{ borderColor: 'var(--border)' }}>
            <div>
              <p style={{ color: 'var(--text)' }}>{j.name ?? j.job_id.slice(0, 8)}</p>
              <p style={{ color: 'var(--muted)' }}>{j.cron_expr ?? (j.every_seconds ? `every ${j.every_seconds}s` : j.at)}</p>
              <p style={{ color: 'var(--muted)' }} className="truncate max-w-xs">{j.message}</p>
            </div>
            <div className="flex gap-2 ml-4 shrink-0">
              <button onClick={() => toggleJob(j)} className="text-xs" style={{ color: j.enabled ? 'var(--accent)' : 'var(--muted)' }}>
                {j.enabled ? 'enabled' : 'disabled'}
              </button>
              <button onClick={() => deleteJob(j.job_id)} className="text-xs text-red-400 hover:text-red-300">delete</button>
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
                <input type="radio" checked={form.scheduleType === t} onChange={() => setForm(f => ({ ...f, scheduleType: t }))} />
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
      <h2 className="text-xs uppercase tracking-widest font-semibold" style={{ color: 'var(--muted)' }}>{title}</h2>
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

function FormField({ label, value, onChange, type = 'text' }: { label: string; value: string; onChange: (v: string) => void; type?: string }) {
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

function SaveButton({ onClick, label = 'Save', loading }: { onClick: () => void; label?: string; loading: boolean }) {
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
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Manual validation — navigate to /settings**

Verify in browser:
- 5 tabs render and switch correctly
- Profile tab loads email, soul, and memory from API
- Devices tab lists tokens and devices; expand shows policy + MCP editor
- Channels tab shows Discord/Telegram forms
- Skills tab lists skills; install and paste forms work
- Cron tab shows jobs; create form toggles schedule type fields

- [ ] **Step 4: Commit**

```bash
git add src/pages/Settings.tsx
git commit -m "feat(frontend): Settings page — Profile, Devices, Channels, Skills, Cron tabs"
```

---

## Task 15: Admin page

**Files:**
- Modify: `plexus-frontend/src/pages/Admin.tsx`

- [ ] **Step 1: Write Admin.tsx**

```tsx
import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { api } from '../lib/api'
import type { LlmConfig, RateLimit, DefaultSoul, McpServerEntry } from '../lib/types'

type Tab = 'llm' | 'soul' | 'rate' | 'mcp'

export default function Admin() {
  const [tab, setTab] = useState<Tab>('llm')
  const navigate = useNavigate()

  const tabs: { id: Tab; label: string }[] = [
    { id: 'llm', label: 'LLM' },
    { id: 'soul', label: 'Default Soul' },
    { id: 'rate', label: 'Rate Limit' },
    { id: 'mcp', label: 'Server MCP' },
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

        {/* Tab bar */}
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
      </div>
    </div>
  )
}

// ── LLM Tab ───────────────────────────────────────────────────────────────────

function LlmTab() {
  const [form, setForm] = useState<LlmConfig>({ api_base: '', model: '', api_key: '', context_window: 128000 })
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
      <AdminField label="API Base URL" value={form.api_base} onChange={v => setForm(f => ({ ...f, api_base: v }))} placeholder="https://api.openai.com/v1" />
      <AdminField label="Model" value={form.model} onChange={v => setForm(f => ({ ...f, model: v }))} placeholder="gpt-4o" />
      <AdminField label="API Key" value={form.api_key} onChange={v => setForm(f => ({ ...f, api_key: v }))} type="password" placeholder="sk-..." />
      <AdminField label="Context Window (tokens)" value={String(form.context_window)} onChange={v => setForm(f => ({ ...f, context_window: parseInt(v) || 128000 }))} type="number" />
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
        className="w-full rounded-lg p-3 text-sm font-mono resize-y outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        placeholder="You are a helpful AI assistant."
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
    api.get<{ mcp_servers: McpServerEntry[] }>('/api/server-mcp')
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
        Admin-configured MCP servers shared across all users. Use lightweight, API-proxy style servers only (ADR-27).
      </p>
      <textarea
        value={json}
        onChange={e => setJson(e.target.value)}
        rows={16}
        className="w-full rounded-lg p-3 text-xs font-mono resize-y outline-none border"
        style={{ background: 'var(--bg)', color: 'var(--text)', borderColor: 'var(--border)' }}
        placeholder='[{"name":"minimax","command":"uvx","args":["minimax-mcp"],"env":{"MINIMAX_API_KEY":"..."},"enabled":true}]'
      />
      <AdminSave onClick={save} loading={loading} />
      {msg && <p className="text-xs" style={{ color: 'var(--accent)' }}>{msg}</p>}
    </div>
  )
}

// ── Shared primitives ─────────────────────────────────────────────────────────

function AdminField({ label, value, onChange, type = 'text', placeholder }: {
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
```

- [ ] **Step 2: Type-check**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Manual validation — navigate to /admin**

Log in as admin. Navigate to `/admin`. Verify:
- LLM tab: loads existing config, saves correctly
- Default Soul tab: loads and saves global soul
- Rate Limit tab: loads and saves limit (0 = unlimited)
- Server MCP tab: loads JSON array, saves to server, MCP restarts

- [ ] **Step 4: Commit**

```bash
git add src/pages/Admin.tsx
git commit -m "feat(frontend): Admin page — LLM, Default Soul, Rate Limit, Server MCP tabs"
```

---

## Task 16: Build verification

**Files:** none (verification only)

- [ ] **Step 1: Run type-check (clean)**

```bash
cd plexus-frontend && npm run typecheck
```

Expected: zero errors.

- [ ] **Step 2: Build for production**

```bash
cd plexus-frontend && npm run build
```

Expected output similar to:
```
✓ built in Xs
dist/index.html        X kB
dist/assets/index-XXX.js   XXX kB
dist/assets/index-XXX.css    X kB
```

No errors. Warnings about bundle size are OK.

- [ ] **Step 3: Verify gateway serves the dist**

Make sure `PLEXUS_FRONTEND_DIR` in `plexus-gateway/.env` points to the correct absolute or relative path:

```
PLEXUS_FRONTEND_DIR=../plexus-frontend/dist
```

Restart the gateway:
```bash
cd Plexus && cargo run --package plexus-gateway
```

Open `http://localhost:9090` — expect the Plexus login page (not a 404). Verify login, chat, settings, and admin all work through the gateway URL (port 9090, not 5173).

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat(frontend): M3b complete — production build verified"
```
