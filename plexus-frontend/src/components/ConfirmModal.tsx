type Props = {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  destructive?: boolean;
};

export function ConfirmModal({
  open,
  title,
  message,
  confirmLabel = 'Confirm',
  onConfirm,
  onCancel,
  destructive,
}: Props) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={onCancel}
    >
      <div
        className="rounded p-6 max-w-md w-full"
        style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold mb-2">{title}</h2>
        <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
          {message}
        </p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="text-sm px-3 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="text-sm px-3 py-1 rounded"
            style={{
              background: destructive ? '#ef4444' : 'var(--accent)',
              color: 'var(--bg)',
            }}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
