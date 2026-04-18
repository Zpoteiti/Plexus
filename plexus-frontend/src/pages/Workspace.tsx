import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeft } from 'lucide-react';
import { api } from '../lib/api';
import type { WorkspaceFile, WorkspaceQuota } from '../lib/types';

export default function Workspace() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const selectedPath = params.get('path') ?? '';

  // tree rendered in B-9; fetched here so the API call is wired up from the start
  const [_tree, setTree] = useState<WorkspaceFile[] | null>(null);
  const [quota, setQuota] = useState<WorkspaceQuota | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    try {
      const [t, q] = await Promise.all([
        api.get<WorkspaceFile[]>('/api/workspace/tree'),
        api.get<WorkspaceQuota>('/api/workspace/quota'),
      ]);
      setTree(t);
      setQuota(q);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed to load workspace');
    }
  }

  return (
    <div className="flex flex-col h-screen" style={{ background: 'var(--bg)', color: 'var(--text)' }}>
      <header
        className="flex items-center gap-4 px-4 py-3 border-b"
        style={{ borderColor: 'var(--border)' }}
      >
        <button onClick={() => navigate('/settings')} className="hover:opacity-70">
          <ArrowLeft size={18} />
        </button>
        <h1 className="text-lg font-semibold">Workspace</h1>
        <div className="ml-auto">
          {quota && (
            <span className="text-sm" style={{ color: 'var(--muted)' }}>
              {formatBytes(quota.used_bytes)} / {formatBytes(quota.total_bytes)}
            </span>
          )}
        </div>
      </header>

      {error && (
        <div className="p-2 text-sm" style={{ color: '#ff6b6b' }}>
          {error}
        </div>
      )}

      <main className="flex-1 flex overflow-hidden">
        <aside
          className="w-1/4 overflow-y-auto border-r p-2"
          style={{ borderColor: 'var(--border)', background: 'var(--sidebar)' }}
        >
          {/* Tree will land in B-9 */}
          <div style={{ color: 'var(--muted)' }}>Tree pane (B-9)</div>
        </aside>

        <section className="flex-1 overflow-y-auto p-4">
          {/* Content pane will land in B-10 */}
          <div style={{ color: 'var(--muted)' }}>
            Content pane for <code>{selectedPath || '(nothing selected)'}</code> (B-10)
          </div>
        </section>
      </main>
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
