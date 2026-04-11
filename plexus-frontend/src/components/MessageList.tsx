import { useEffect, useRef } from 'react'
import type { ChatMessage } from '../lib/types'
import Message from './Message'
import ProgressHint from './ProgressHint'

interface Props {
  messages: ChatMessage[]
  progressHint: string | null
}

export default function MessageList({ messages, progressHint }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages.length, progressHint])

  return (
    <div className="flex-1 overflow-y-auto px-4 py-4">
      {messages.map(msg => (
        <Message key={msg.id} message={msg} />
      ))}
      {progressHint && <ProgressHint hint={progressHint} />}
      <div ref={bottomRef} />
    </div>
  )
}
