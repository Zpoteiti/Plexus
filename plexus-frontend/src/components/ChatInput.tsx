import { useState, useRef, KeyboardEvent, ClipboardEvent, DragEvent } from 'react'
import { Send, Paperclip, X } from 'lucide-react'
import { uploadFile, MAX_UPLOAD_BYTES, UploadResult } from '../lib/upload'

interface Chip {
  key: string
  file: File
  progress: number
  fileId?: string
  error?: string
  controller: AbortController
}

interface Props {
  onSend: (content: string, media: string[]) => void
  disabled?: boolean
}

export default function ChatInput({ onSend, disabled }: Props) {
  const [value, setValue] = useState('')
  const [chips, setChips] = useState<Chip[]>([])
  const [isDragging, setDragging] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  const anyUploading = chips.some(c => c.progress < 100 && !c.error)

  function updateChip(key: string, patch: Partial<Chip>) {
    setChips(prev => prev.map(c => (c.key === key ? { ...c, ...patch } : c)))
  }

  function removeChip(key: string) {
    setChips(prev => {
      const c = prev.find(x => x.key === key)
      c?.controller.abort()
      return prev.filter(x => x.key !== key)
    })
  }

  function addFiles(files: FileList | File[]) {
    const arr = Array.from(files)
    for (const file of arr) {
      if (file.size > MAX_UPLOAD_BYTES) {
        alert(`${file.name} exceeds 20 MB limit`)
        continue
      }
      const key = `${Date.now()}-${Math.random()}`
      const controller = new AbortController()
      const chip: Chip = { key, file, progress: 0, controller }
      setChips(prev => [...prev, chip])
      uploadFile(
        file,
        pct => updateChip(key, { progress: pct }),
        controller.signal,
      )
        .then((res: UploadResult) => {
          updateChip(key, { progress: 100, fileId: res.file_id })
        })
        .catch((e: Error) => {
          updateChip(key, { error: e.message })
        })
    }
  }

  function submit() {
    const trimmed = value.trim()
    const media = chips
      .filter(c => c.fileId && !c.error)
      .map(c => `/api/files/${c.fileId}`)
    if ((!trimmed && media.length === 0) || disabled || anyUploading) return
    onSend(trimmed, media)
    setValue('')
    setChips([])
    if (textareaRef.current) textareaRef.current.style.height = 'auto'
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

  function handlePaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    const items = e.clipboardData?.items
    if (!items) return
    const files: File[] = []
    for (let i = 0; i < items.length; i++) {
      const item = items[i]
      if (item.kind === 'file') {
        const f = item.getAsFile()
        if (f) files.push(f)
      }
    }
    if (files.length > 0) {
      e.preventDefault()
      addFiles(files)
    }
  }

  function handleDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragging(false)
    if (e.dataTransfer.files) addFiles(e.dataTransfer.files)
  }

  function handleDragOver(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragging(true)
  }

  function handleDragLeave() {
    setDragging(false)
  }

  return (
    <div
      className={`flex flex-col gap-2 rounded-xl border p-3 ${isDragging ? 'ring-2' : ''}`}
      style={{
        background: 'var(--card)',
        borderColor: 'var(--border)',
        width: 'min(90vw, 720px)',
        minWidth: 'min(90vw, 420px)',
      }}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {chips.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {chips.map(c => (
            <div
              key={c.key}
              className="flex items-center gap-1 px-2 py-1 rounded text-xs"
              style={{ background: 'var(--muted)' }}
              title={c.error || `${c.progress}%`}
            >
              <span style={{ color: 'var(--text)' }}>{c.file.name}</span>
              {c.error ? (
                <span style={{ color: '#ef4444' }}>⚠</span>
              ) : c.progress < 100 ? (
                <span style={{ color: 'var(--muted-fg)' }}>{c.progress}%</span>
              ) : (
                <span style={{ color: 'var(--accent)' }}>✓</span>
              )}
              <button onClick={() => removeChip(c.key)} title="Remove" style={{ color: 'var(--muted-fg)' }}>
                <X size={12} />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex items-end gap-2">
        <button
          onClick={() => fileInputRef.current?.click()}
          disabled={disabled}
          className="p-1.5 rounded-lg disabled:opacity-30"
          style={{ color: 'var(--muted-fg)' }}
          title="Attach file"
        >
          <Paperclip size={16} />
        </button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          hidden
          onChange={e => {
            if (e.target.files) addFiles(e.target.files)
            e.target.value = ''
          }}
        />

        <textarea
          ref={textareaRef}
          value={value}
          onChange={e => setValue(e.target.value)}
          onInput={handleInput}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          disabled={disabled}
          placeholder="Message Plexus… (Enter to send, Shift+Enter for newline)"
          rows={1}
          className="flex-1 resize-none outline-none text-sm bg-transparent"
          style={{ color: 'var(--text)', maxHeight: 200 }}
        />

        <button
          onClick={submit}
          disabled={
            disabled ||
            anyUploading ||
            (!value.trim() && chips.filter(c => c.fileId && !c.error).length === 0)
          }
          className="p-1.5 rounded-lg transition-all disabled:opacity-30"
          style={{ color: 'var(--accent)' }}
          title={anyUploading ? 'Waiting for uploads' : 'Send'}
        >
          <Send size={16} />
        </button>
      </div>
    </div>
  )
}
