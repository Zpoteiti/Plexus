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
      {/* Toggle */}
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
              <MessageSquare
                size={12}
                style={{ flexShrink: 0, color: isActive ? 'var(--accent)' : 'var(--muted)' }}
              />
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
