import { useEffect, useState } from 'react'
import type { ChatMessage, ContentBlock, ImageBlock } from '../lib/types'
import { useAuthStore } from '../store/auth'
import MarkdownContent from './MarkdownContent'

interface Props {
  message: ChatMessage
}

/**
 * Auth'd image loader. `<img src>` can't carry an Authorization header, so we
 * fetch the bytes with the JWT, turn the response into a blob URL, and swap
 * that in as the src. On 404 (post-TTL) or any other error we fall back to
 * the embedded base64 data URL. Same pattern as Workspace.tsx.
 */
function ImageBlockView({ block }: { block: ImageBlock }) {
  const base64Url = `data:${block.source.media_type};base64,${block.source.data}`
  const [src, setSrc] = useState<string>(base64Url)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    if (!block.workspace_path) return
    const token = useAuthStore.getState().token
    if (!token) return

    const controller = new AbortController()
    let objectUrl: string | null = null
    let cancelled = false

    const url = `/api/workspace/files/${block.workspace_path}`
    fetch(url, {
      headers: { Authorization: `Bearer ${token}` },
      signal: controller.signal,
    })
      .then(async (r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        const buf = await r.arrayBuffer()
        if (cancelled) return
        const mime = r.headers.get('Content-Type') ?? block.source.media_type
        objectUrl = URL.createObjectURL(new Blob([buf], { type: mime }))
        setSrc(objectUrl)
        setLoaded(true)
      })
      .catch(() => {
        // Workspace fetch failed (most likely 404 post-TTL sweep, or auth);
        // base64 fallback is already in state, nothing to do.
      })

    return () => {
      cancelled = true
      controller.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [block.workspace_path, block.source.media_type, block.source.data])

  return (
    <img
      src={src}
      alt={block.filename ?? ''}
      className="max-w-xs rounded border"
      style={{ borderColor: 'var(--border)', maxHeight: 240 }}
      onError={() => {
        // If even the base64 fails, we've got nothing left.
        if (loaded) setSrc(base64Url)
      }}
    />
  )
}

function BlockView({ block }: { block: ContentBlock }) {
  if (block.type === 'text') {
    return <MarkdownContent content={block.text} />
  }
  return <ImageBlockView block={block} />
}

function MessageBody({ content }: { content: string | ContentBlock[] }) {
  if (typeof content === 'string') {
    return <MarkdownContent content={content} />
  }
  return (
    <div className="flex flex-col gap-2">
      {content.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
    </div>
  )
}

export default function Message({ message }: Props) {
  const isUser = message.role === 'user'

  if (isUser) {
    // User bubbles render plain text (not markdown-rendered) per existing
    // styling; images inline below the text.
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
          {typeof message.content === 'string' ? (
            message.content && <div>{message.content}</div>
          ) : (
            <div className="flex flex-col gap-2">
              {message.content.map((b, i) =>
                b.type === 'text' ? (
                  b.text ? <div key={i}>{b.text}</div> : null
                ) : (
                  <ImageBlockView key={i} block={b} />
                ),
              )}
            </div>
          )}
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
        <MessageBody content={message.content} />
      </div>
    </div>
  )
}
