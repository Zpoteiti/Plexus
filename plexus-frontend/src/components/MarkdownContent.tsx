import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism'
import type { Components } from 'react-markdown'

interface Props {
  content: string
}

const components: Components = {
  code({ className, children, ...props }) {
    const match = /language-(\w+)/.exec(className ?? '')
    const isBlock = Boolean(match)
    if (isBlock) {
      return (
        <SyntaxHighlighter
          style={vscDarkPlus}
          language={match![1]}
          PreTag="div"
          customStyle={{
            borderRadius: 8,
            fontSize: 12,
            marginTop: 8,
            marginBottom: 8,
            background: '#0d1117',
            border: '1px solid #1a2332',
          }}
        >
          {String(children).replace(/\n$/, '')}
        </SyntaxHighlighter>
      )
    }
    return (
      <code
        className={className}
        style={{
          color: '#39ff14',
          background: '#161b22',
          padding: '1px 5px',
          borderRadius: 4,
          fontSize: 12,
        }}
        {...props}
      >
        {children}
      </code>
    )
  },
  a({ href, children }) {
    return (
      <a
        href={href}
        target="_blank"
        rel="noreferrer"
        style={{ color: '#39ff14', textDecoration: 'underline' }}
      >
        {children}
      </a>
    )
  },
}

export default function MarkdownContent({ content }: Props) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {content}
    </ReactMarkdown>
  )
}
