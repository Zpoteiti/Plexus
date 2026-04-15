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
  const token = useAuthStore(s => s.token)
  const userId = useAuthStore(s => s.userId)
  const displayName = useAuthStore(s => s.displayName)
  const refreshProfile = useAuthStore(s => s.refreshProfile)
  const {
    init,
    loadSessions,
    loadMessages,
    sendMessage,
    setCurrentSession,
    messagesBySession,
    progressBySession,
    wsStatus,
  } = useChatStore()
  const startPolling = useDevicesStore(s => s.startPolling)

  // Generate session ID if URL has none
  useEffect(() => {
    if (!sessionId && userId) {
      navigate(`/chat/gateway:${userId}:${crypto.randomUUID()}`, { replace: true })
    }
  }, [sessionId, userId, navigate])

  // Init WS and stores once on mount
  useEffect(() => {
    if (!token) return
    init()
    wsManager.connect(token)
    void loadSessions()
    void refreshProfile()
    const stopPolling = startPolling()
    return stopPolling
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

  // Non-gateway sessions (discord:…, telegram:…, cron:…) are read-only in the browser
  const channel = sessionId?.split(':')[0] ?? 'gateway'
  const isReadOnly = channel !== 'gateway'

  const channelLabel: Record<string, string> = {
    discord: 'Discord',
    telegram: 'Telegram',
    cron: 'Cron',
  }

  function handleSend(content: string, media: string[]) {
    if (!sessionId || isReadOnly) return
    sendMessage(sessionId, content, media)
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
                Hey, {displayName ?? userId?.slice(0, 8) ?? 'there'}
              </p>
              <p className="text-sm mt-1" style={{ color: 'var(--muted)' }}>
                {isConnected ? 'What can I help you with?' : 'Connecting to Plexus…'}
              </p>
            </div>
            {!isReadOnly && <ChatInput onSend={handleSend} disabled={!isConnected} />}
          </div>
        ) : (
          /* Active state */
          <>
            <MessageList messages={messages} progressHint={progress} />
            <div className="flex justify-center pb-4 pt-2">
              {isReadOnly ? (
                <p className="text-xs px-4 py-2 rounded-lg" style={{ color: 'var(--muted)', border: '1px solid var(--border)' }}>
                  Read-only — send messages via {channelLabel[channel] ?? channel}
                </p>
              ) : (
                <ChatInput onSend={handleSend} disabled={!isConnected} />
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
