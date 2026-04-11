import { useState, useRef, KeyboardEvent } from 'react'
import { Send } from 'lucide-react'

interface Props {
  onSend: (content: string) => void
  disabled?: boolean
}

export default function ChatInput({ onSend, disabled }: Props) {
  const [value, setValue] = useState('')
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  function submit() {
    const trimmed = value.trim()
    if (!trimmed || disabled) return
    onSend(trimmed)
    setValue('')
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto'
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
    }
  }

  function handleInput() {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 200) + 'px'
  }

  return (
    <div
      className="flex items-end gap-2 rounded-xl border p-3"
      style={{
        background: 'var(--card)',
        borderColor: 'var(--border)',
        width: 'min(90vw, 720px)',
        minWidth: 'min(90vw, 420px)',
      }}
    >
      <textarea
        ref={textareaRef}
        value={value}
        onChange={e => setValue(e.target.value)}
        onInput={handleInput}
        onKeyDown={handleKeyDown}
        disabled={disabled}
        placeholder="Message Plexus… (Enter to send, Shift+Enter for newline)"
        rows={1}
        className="flex-1 resize-none outline-none text-sm bg-transparent"
        style={{ color: 'var(--text)', maxHeight: 200 }}
      />
      <button
        onClick={submit}
        disabled={disabled || !value.trim()}
        className="p-1.5 rounded-lg transition-all disabled:opacity-30"
        style={{ color: 'var(--accent)' }}
        title="Send"
      >
        <Send size={16} />
      </button>
    </div>
  )
}
