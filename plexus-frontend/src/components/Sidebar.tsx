import { useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { MessageSquare, Plus, ChevronLeft, ChevronRight, Settings, Shield, Trash2, X } from 'lucide-react'
import { useChatStore } from '../store/chat'
import { useAuthStore } from '../store/auth'

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const [selectMode, setSelectMode] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [deleting, setDeleting] = useState(false)

  const { sessionId: activeId } = useParams<{ sessionId: string }>()
  const sessions = useChatStore(s => s.sessions)
  const deleteSessions = useChatStore(s => s.deleteSessions)
  const isAdmin = useAuthStore(s => s.isAdmin)
  const navigate = useNavigate()

  function newSession() {
    const userId = useAuthStore.getState().userId ?? 'unknown'
    navigate(`/chat/gateway:${userId}:${crypto.randomUUID()}`)
  }

  function toggleSelect(id: string) {
    setSelected(prev => {
      const next = new Set(prev)
      next.has(id) ? next.delete(id) : next.add(id)
      return next
    })
  }

  function exitSelectMode() {
    setSelectMode(false)
    setSelected(new Set())
  }

  async function handleDelete() {
    if (selected.size === 0) return
    setDeleting(true)
    const ids = [...selected]
    await deleteSessions(ids)
    // Navigate away if active session was deleted
    if (activeId && ids.includes(activeId)) {
      navigate('/chat', { replace: true })
    }
    exitSelectMode()
    setDeleting(false)
  }

  return (
    <aside
      className="flex flex-col h-full border-r shrink-0 transition-all duration-200"
      style={{ width: collapsed ? 48 : 200, background: 'var(--sidebar)', borderColor: 'var(--border)' }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-2 py-3 border-b" style={{ borderColor: 'var(--border)' }}>
        {!collapsed && (
          <span className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--accent)' }}>
            Plexus
          </span>
        )}
        <button
          onClick={() => { setCollapsed(c => !c); if (!collapsed) exitSelectMode() }}
          className="p-1 rounded hover:bg-[#1a2332] transition-colors ml-auto"
          style={{ color: 'var(--muted)' }}
        >
          {collapsed ? <ChevronRight size={14} /> : <ChevronLeft size={14} />}
        </button>
      </div>

      {/* New chat / select mode toolbar */}
      <div className="px-2 py-2 border-b" style={{ borderColor: 'var(--border)' }}>
        {!selectMode ? (
          <div className="flex items-center gap-1">
            <button
              onClick={newSession}
              className="flex items-center gap-2 flex-1 rounded-lg px-2 py-1.5 text-xs transition-colors hover:bg-[#1a2332]"
              style={{ color: 'var(--accent)' }}
              title="New chat"
            >
              <Plus size={14} />
              {!collapsed && <span>New chat</span>}
            </button>
            {!collapsed && sessions.length > 0 && (
              <button
                onClick={() => setSelectMode(true)}
                className="p-1.5 rounded hover:bg-[#1a2332] transition-colors"
                style={{ color: 'var(--muted)' }}
                title="Select to delete"
              >
                <Trash2 size={13} />
              </button>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-1">
            <button
              onClick={handleDelete}
              disabled={selected.size === 0 || deleting}
              className="flex items-center gap-1.5 flex-1 rounded-lg px-2 py-1.5 text-xs transition-colors disabled:opacity-40"
              style={{
                color: selected.size > 0 ? '#ef4444' : 'var(--muted)',
                background: selected.size > 0 ? 'rgba(239,68,68,0.08)' : 'transparent',
              }}
            >
              <Trash2 size={13} />
              <span>{deleting ? 'Deleting…' : `Delete${selected.size > 0 ? ` (${selected.size})` : ''}`}</span>
            </button>
            <button
              onClick={exitSelectMode}
              className="p-1.5 rounded hover:bg-[#1a2332] transition-colors"
              style={{ color: 'var(--muted)' }}
              title="Cancel"
            >
              <X size={13} />
            </button>
          </div>
        )}
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto py-1">
        {sessions.map(session => {
          const isActive = session.session_id === activeId
          const isChecked = selected.has(session.session_id)
          const shortId = session.session_id.split(':')[2]?.slice(0, 8) ?? session.session_id

          return (
            <div
              key={session.session_id}
              onClick={() => selectMode ? toggleSelect(session.session_id) : navigate(`/chat/${session.session_id}`)}
              className="flex items-center gap-2 w-full px-3 py-2 text-xs cursor-pointer transition-colors hover:bg-[#1a2332] rounded"
              style={{
                color: isActive && !selectMode ? 'var(--accent)' : 'var(--text)',
                background: isChecked
                  ? 'rgba(239,68,68,0.08)'
                  : isActive && !selectMode ? 'rgba(57,255,20,0.06)' : 'transparent',
              }}
              title={session.session_id}
            >
              {selectMode && !collapsed ? (
                <input
                  type="checkbox"
                  checked={isChecked}
                  onChange={() => toggleSelect(session.session_id)}
                  onClick={e => e.stopPropagation()}
                  className="shrink-0 cursor-pointer"
                  style={{ accentColor: '#ef4444', width: 12, height: 12 }}
                />
              ) : (
                <MessageSquare
                  size={12}
                  style={{ flexShrink: 0, color: isActive && !selectMode ? 'var(--accent)' : 'var(--muted)' }}
                />
              )}
              {!collapsed && <span className="truncate flex-1">{shortId}</span>}
              {!collapsed && session.hasUnread && !isActive && (
                <span
                  className="w-2 h-2 rounded-full shrink-0"
                  style={{ background: 'var(--accent)' }}
                />
              )}
            </div>
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
