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
