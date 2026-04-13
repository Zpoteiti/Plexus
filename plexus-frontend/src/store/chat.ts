import { create } from 'zustand'
import { api } from '../lib/api'
import { wsManager, WsStatus } from '../lib/ws'
import type { Session, ChatMessage, ApiMessage } from '../lib/types'

interface ChatState {
  sessions: Session[]
  currentSessionId: string | null
  messagesBySession: Record<string, ChatMessage[]>
  restLoadedSessions: Record<string, true>
  progressBySession: Record<string, string | null>
  wsStatus: WsStatus

  init: () => void
  loadSessions: () => Promise<void>
  loadMessages: (sessionId: string) => Promise<void>
  deleteSessions: (sessionIds: string[]) => Promise<void>
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
  restLoadedSessions: {},
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
        store.handleIncomingMessage(
          sessionId,
          data['content'] as string,
          data['media'] as string[] | undefined,
        )
      } else if (data['type'] === 'progress') {
        store.setProgressHint(sessionId, data['content'] as string)
      } else if (data['type'] === 'error') {
        store.handleError((data['reason'] as string | undefined) ?? 'Unknown error')
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

  deleteSessions: async (sessionIds) => {
    await Promise.all(sessionIds.map(id => api.delete(`/api/sessions/${id}`).catch(() => {})))
    set(s => ({
      sessions: s.sessions.filter(s => !sessionIds.includes(s.session_id)),
      messagesBySession: Object.fromEntries(
        Object.entries(s.messagesBySession).filter(([id]) => !sessionIds.includes(id))
      ),
    }))
  },

  loadMessages: async (sessionId) => {
    // Guard: only load once per session per page load
    if (get().restLoadedSessions[sessionId]) return

    const apiMsgs = await api.get<ApiMessage[]>(
      `/api/sessions/${sessionId}/messages?limit=200`,
    )

    // Filter to only user and assistant text messages — tool calls and tool
    // results are internal LLM context and should never be shown in the UI
    const msgs: ChatMessage[] = apiMsgs
      .filter(m => m.role === 'user' || (m.role === 'assistant' && !m.tool_name && m.content?.trim()))
      .map(m => ({
        id: m.message_id,
        session_id: m.session_id,
        role: m.role as 'user' | 'assistant',
        content: m.content,
        created_at: m.created_at,
      }))

    set(s => {
      const existing = s.messagesBySession[sessionId] ?? []
      // Prepend REST history; preserve any WS messages that arrived during fetch
      const merged = [...msgs, ...existing].sort(
        (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
      )
      return {
        messagesBySession: { ...s.messagesBySession, [sessionId]: merged },
        restLoadedSessions: { ...s.restLoadedSessions, [sessionId]: true as const },
      }
    })
  },

  setCurrentSession: (sessionId) => {
    set(s => ({
      currentSessionId: sessionId,
      // Clear progress hint for the previous session on switch
      progressBySession: s.currentSessionId
        ? { ...s.progressBySession, [s.currentSessionId]: null }
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
    console.warn('[plexus] WS error:', reason)
  },
}))

// Cleanup for logout / tests
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
    restLoadedSessions: {},
    progressBySession: {},
    wsStatus: 'closed',
  })
}
