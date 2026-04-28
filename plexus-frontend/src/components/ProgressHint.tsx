interface Props {
  hint: string
}

export default function ProgressHint({ hint }: Props) {
  return (
    <div className="flex items-center gap-2 px-2 py-1 text-xs" style={{ color: 'var(--muted)' }}>
      <span
        style={{
          width: 8,
          height: 8,
          display: 'inline-block',
          border: '2px solid transparent',
          borderTopColor: 'var(--accent)',
          borderRadius: '50%',
          animation: 'spin 0.8s linear infinite',
          flexShrink: 0,
        }}
      />
      <span className="truncate">{hint}</span>
    </div>
  )
}
