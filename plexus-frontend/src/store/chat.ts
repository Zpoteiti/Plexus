import { create } from 'zustand'
import { api } from '../lib/api'
import { wsManager, WsStatus } from '../lib/ws'
import { parseContentBlocks } from '../lib/content-blocks'
import type { UploadedAttachment } from '../lib/upload'
import type {
  Session,
  ChatMessage,
  ApiMessage,
  ContentBlock,
} from '../lib/types'

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
  refreshSession: (sessionId: string) => Promise<void>
  deleteSessions: (sessionIds: string[]) => Promise<void>
  setCurrentSession: (sessionId: string | null) => void
  sendMessage: (
    sessionId: string,
    content: string,
    attachments?: UploadedAttachment[],
  ) => void
  handleIncomingMessage: (sessionId: string, content: string) => void
  setProgressHint: (sessionId: string, hint: string) => void
  clearProgress: (sessionId: string) => void
  handleError: (reason: string) => void
}

// Module-level init guard (survives React StrictMode double-invocations)
let initialized = false
let unsubMsg: (() => void) | null = null
let unsubStatus: (() => void) | null = null

/**
 * Build Anthropic-style content blocks from a user's text + attachments.
 * Images embed base64 data so history survives the 30-day `.attachments/`
 * TTL sweep; workspace_path is carried alongside so re-renders can prefer
 * the cacheable URL while the file still exists. Spec §2.1.
 */
function buildContentBlocks(
  text: string,
  attachments: UploadedAttachment[],
): ContentBlock[] {
  const blocks: ContentBlock[] = []
  if (text) blocks.push({ type: 'text', text })
  for (const a of attachments) {
    if (a.media_type.startsWith('image/')) {
      blocks.push({
        type: 'image',
        source: {
          type: 'base64',
          media_type: a.media_type,
          data: a.base64_data,
        },
        workspace_path: a.workspace_path,
        filename: a.filename,
      })
    } else {
      // Non-image attachment: render as a bracketed reference in text.
      // Non-images on chat-drop are out of scope for FR2 (spec §2.1 covers
      // images; file_transfer is the path for non-images on the agent side).
      blocks.push({
        type: 'text',
        text: `[Attachment: ${a.filename} → /api/workspace/files/${a.workspace_path}]`,
      })
    }
  }
  return blocks
}

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
        )
      } else if (data['type'] === 'progress') {
        store.setProgressHint(sessionId, data['content'] as string)
      } else if (data['type'] === 'error') {
        store.handleError((data['reason'] as string | undefined) ?? 'Unknown error')
      } else if (data['type'] === 'session_update') {
        // Server tells us a session has new content. Re-fetch via REST so
        // the UI picks up anything the browser doesn't have yet.
        void store.refreshSession(sessionId)
        // Mark as unread only when the user is NOT already viewing this session.
        if (sessionId !== get().currentSessionId) {
          set(s => ({
            sessions: s.sessions.map(sess =>
              sess.session_id === sessionId ? { ...sess, hasUnread: true } : sess,
            ),
          }))
        }
      }
    })

    unsubStatus = wsManager.onStatus((status) => {
      set({ wsStatus: status })
    })

    set({ wsStatus: wsManager.getStatus() })
  },

  loadSessions: async () => {
    const raw = await api.get<Omit<Session, 'hasUnread'>[]>('/api/sessions')
    const sessions: Session[] = raw.map(s => ({ ...s, hasUnread: false }))
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
    // results are internal LLM context and should never be shown in the UI.
    //
    // `ApiMessage.content` is always a string on the wire, but user messages
    // with attachments were persisted as a JSON-stringified
    // `Content::Blocks` array (spec §2.1). Run the detect+parse helper so
    // image blocks route through the ContentBlock[] renderer instead of
    // showing raw JSON (FR2b).
    const msgs: ChatMessage[] = apiMsgs
      .filter(m => m.role === 'user' || (m.role === 'assistant' && !m.tool_name && m.content?.trim()))
      .map(m => ({
        id: m.message_id,
        session_id: m.session_id,
        role: m.role as 'user' | 'assistant',
        content: parseContentBlocks(m.content),
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

  refreshSession: async (sessionId) => {
    // Bypass the restLoadedSessions guard so new messages get picked up.
    set(s => ({
      restLoadedSessions: Object.fromEntries(
        Object.entries(s.restLoadedSessions).filter(([id]) => id !== sessionId),
      ),
    }))
    await get().loadMessages(sessionId)
  },

  setCurrentSession: (sessionId) => {
    if (get().currentSessionId === sessionId) return
    set(s => ({
      currentSessionId: sessionId,
      // Clear progress hint for the previous session on switch
      progressBySession: s.currentSessionId
        ? { ...s.progressBySession, [s.currentSessionId]: null }
        : s.progressBySession,
      // Clear unread badge for the session the user is now viewing
      sessions: sessionId
        ? s.sessions.map(sess =>
            sess.session_id === sessionId ? { ...sess, hasUnread: false } : sess,
          )
        : s.sessions,
    }))
  },

  sendMessage: (sessionId, content, attachments) => {
    const hasAttachments = !!attachments && attachments.length > 0

    // Build Anthropic-style content blocks for the local optimistic echo.
    // These are also what we'd send to the server once it accepts them on
    // the wire; today the server reconstructs equivalent blocks from the
    // `media` workspace-path array (see spec §2.1 transitional state).
    const localContent: string | ContentBlock[] = hasAttachments
      ? buildContentBlocks(content, attachments!)
      : content

    const localMsg: ChatMessage = {
      id: crypto.randomUUID(),
      session_id: sessionId,
      role: 'user',
      content: localContent,
      created_at: new Date().toISOString(),
    }
    set(s => ({
      messagesBySession: {
        ...s.messagesBySession,
        [sessionId]: [...(s.messagesBySession[sessionId] ?? []), localMsg],
      },
    }))

    // Wire frame: server currently consumes `content` (string) + `media`
    // (workspace paths). context.rs loads each path via workspace_fs and
    // emits its own ContentBlocks for the LLM. When server-side content-
    // block persistence lands (spec §2.1), we'll switch to sending the
    // full `content_blocks` array inline.
    const media = hasAttachments
      ? attachments!.map(a => a.workspace_path)
      : undefined

    wsManager.send({
      type: 'message',
      session_id: sessionId,
      content,
      ...(media && media.length > 0 ? { media } : {}),
    })
  },

  handleIncomingMessage: (sessionId, content) => {
    const msg: ChatMessage = {
      id: crypto.randomUUID(),
      session_id: sessionId,
      role: 'assistant',
      content,
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
    if (get().progressBySession[sessionId] === hint) return
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
