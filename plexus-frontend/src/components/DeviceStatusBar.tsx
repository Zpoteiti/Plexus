import { useChatStore } from '../store/chat'
import { useDevicesStore } from '../store/devices'

interface Props {
  sessionId: string
}

export default function DeviceStatusBar({ sessionId }: Props) {
  const wsStatus = useChatStore(s => s.wsStatus)
  const devices = useDevicesStore(s => s.devices)

  const wsColor =
    wsStatus === 'open' ? '#39ff14' : wsStatus === 'connecting' ? '#facc15' : '#ef4444'
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
        <span style={{ color: 'var(--muted)' }}>gateway</span>
        <span
          style={{
            width: 7,
            height: 7,
            borderRadius: '50%',
            background: wsColor,
            boxShadow: wsGlow,
            display: 'inline-block',
          }}
        />
      </div>

      {devices.map(d => (
        <div key={d.device_name} className="flex items-center gap-1">
          <span style={{ color: 'var(--muted)' }}>{d.device_name}</span>
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: '50%',
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
