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
