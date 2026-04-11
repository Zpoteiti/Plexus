import { useState, useRef, useEffect } from 'react'
import { ChevronDown } from 'lucide-react'
import { useChatStore } from '../store/chat'
import { useDevicesStore } from '../store/devices'
import type { Device } from '../lib/types'

interface Props {
  sessionId: string
}

export default function DeviceStatusBar({ sessionId }: Props) {
  const wsStatus = useChatStore(s => s.wsStatus)
  const devices = useDevicesStore(s => s.devices)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  // Close on outside click
  useEffect(() => {
    function onPointerDown(e: PointerEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  }, [])

  const shortId = sessionId.split(':')[2]?.slice(0, 8) ?? sessionId

  const gwColor = wsStatus === 'open' ? '#39ff14' : wsStatus === 'connecting' ? '#facc15' : '#ef4444'
  const gwGlow  = wsStatus === 'open' ? '0 0 6px #39ff14' : 'none'

  const onlineCount  = devices.filter(d => d.online).length
  const offlineCount = devices.length - onlineCount
  // Pill color: all online → green, any offline → yellow, no devices → muted
  const pillColor = devices.length === 0
    ? 'var(--muted)'
    : offlineCount === 0 ? '#39ff14' : '#facc15'

  return (
    <div
      className="flex items-center gap-3 px-4 py-2 border-b text-xs select-none"
      style={{ background: 'var(--sidebar)', borderColor: 'var(--border)', color: 'var(--muted)' }}
    >
      {/* Session ID */}
      <span style={{ color: 'var(--text)' }} className="font-mono">{shortId}</span>

      {/* Gateway status — always visible */}
      <div className="flex items-center gap-1 ml-auto">
        <span>gateway</span>
        <Dot color={gwColor} glow={gwGlow} />
      </div>

      {/* Devices dropdown */}
      {devices.length > 0 && (
        <div ref={ref} className="relative">
          <button
            onClick={() => setOpen(o => !o)}
            className="flex items-center gap-1 px-2 py-0.5 rounded transition-colors hover:bg-[#1a2332] cursor-pointer"
            style={{ color: 'var(--muted)', background: 'transparent', border: 'none' }}
          >
            <Dot color={pillColor} />
            <span>{devices.length} device{devices.length !== 1 ? 's' : ''}</span>
            <ChevronDown
              size={11}
              style={{
                transition: 'transform 0.15s',
                transform: open ? 'rotate(180deg)' : 'rotate(0deg)',
              }}
            />
          </button>

          {open && (
            <div
              className="absolute right-0 top-full mt-1 rounded-lg border py-1 z-50 min-w-[160px]"
              style={{ background: 'var(--card)', borderColor: 'var(--border)', boxShadow: '0 4px 16px rgba(0,0,0,0.4)' }}
            >
              {devices.map((d: Device) => (
                <div
                  key={d.device_name}
                  className="flex items-center justify-between gap-3 px-3 py-1.5 text-xs"
                  style={{ color: 'var(--text)' }}
                >
                  <span className="font-mono truncate">{d.device_name}</span>
                  <div className="flex items-center gap-1.5 shrink-0">
                    <span style={{ color: 'var(--muted)' }}>
                      {d.online ? `${d.tool_count} tools` : 'offline'}
                    </span>
                    <Dot
                      color={d.online ? '#39ff14' : '#ef4444'}
                      glow={d.online ? '0 0 6px #39ff14' : 'none'}
                    />
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function Dot({ color, glow }: { color: string; glow?: string }) {
  return (
    <span
      style={{
        display: 'inline-block',
        width: 7,
        height: 7,
        borderRadius: '50%',
        background: color,
        boxShadow: glow ?? 'none',
        flexShrink: 0,
      }}
    />
  )
}

