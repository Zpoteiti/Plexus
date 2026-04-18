import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeft, ChevronRight, ChevronDown, File, Folder } from 'lucide-react';
import { api } from '../lib/api';
import type { WorkspaceFile, WorkspaceQuota } from '../lib/types';

export default function Workspace() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const selectedPath = params.get('path') ?? '';

  const [tree, setTree] = useState<WorkspaceFile[] | null>(null);
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
        <div className="flex-1 max-w-md">
          {quota && <QuotaBar q={quota} />}
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
          {tree && (
            <TreeView
              entries={tree}
              selected={selectedPath}
              onSelect={(path) => setParams({ path })}
            />
          )}
          {!tree && (
            <div style={{ color: 'var(--muted)' }}>Loading…</div>
          )}
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

function QuotaBar({ q }: { q: WorkspaceQuota }) {
  const pct = q.total_bytes > 0 ? (q.used_bytes / q.total_bytes) * 100 : 0;
  const clamped = Math.min(100, pct);
  const color = pct >= 95 ? '#ef4444' : pct >= 80 ? '#f59e0b' : 'var(--accent)';
  return (
    <div className="flex flex-col gap-1">
      <div className="flex justify-between text-xs" style={{ color: 'var(--muted)' }}>
        <span>{formatBytes(q.used_bytes)} / {formatBytes(q.total_bytes)}</span>
        <span>{pct.toFixed(1)}%</span>
      </div>
      <div className="h-2 rounded" style={{ background: 'var(--border)' }}>
        <div
          className="h-2 rounded"
          style={{ width: `${clamped}%`, background: color, transition: 'width 200ms ease' }}
        />
      </div>
      {pct >= 100 && (
        <div className="text-xs" style={{ color: '#ef4444' }}>
          Workspace full. Delete files to resume writes.
        </div>
      )}
    </div>
  );
}

type TreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  size_bytes: number;
  children: TreeNode[];
};

function buildTree(entries: WorkspaceFile[]): TreeNode[] {
  const root: TreeNode = { name: '', path: '', is_dir: true, size_bytes: 0, children: [] };
  // Sort so parents are inserted before children.
  const sorted = [...entries].sort((a, b) => a.path.localeCompare(b.path));
  for (const e of sorted) {
    const parts = e.path.split('/');
    let cur = root;
    for (let i = 0; i < parts.length; i++) {
      const partPath = parts.slice(0, i + 1).join('/');
      let child = cur.children.find((c) => c.path === partPath);
      if (!child) {
        child = {
          name: parts[i],
          path: partPath,
          is_dir: i < parts.length - 1 ? true : e.is_dir,
          size_bytes: i === parts.length - 1 ? e.size_bytes : 0,
          children: [],
        };
        cur.children.push(child);
      }
      cur = child;
    }
  }
  // Sort each level: dirs first, then alphabetical.
  const sortChildren = (n: TreeNode) => {
    n.children.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const c of n.children) sortChildren(c);
  };
  sortChildren(root);
  return root.children;
}

function TreeView({
  entries,
  selected,
  onSelect,
}: {
  entries: WorkspaceFile[];
  selected: string;
  onSelect: (path: string) => void;
}) {
  const tree = buildTree(entries);
  return <TreeNodeList nodes={tree} depth={0} selected={selected} onSelect={onSelect} />;
}

function TreeNodeList({
  nodes,
  depth,
  selected,
  onSelect,
}: {
  nodes: TreeNode[];
  depth: number;
  selected: string;
  onSelect: (path: string) => void;
}) {
  return (
    <ul className="list-none p-0 m-0">
      {nodes.map((n) => (
        <TreeItem key={n.path} node={n} depth={depth} selected={selected} onSelect={onSelect} />
      ))}
    </ul>
  );
}

function TreeItem({
  node,
  depth,
  selected,
  onSelect,
}: {
  node: TreeNode;
  depth: number;
  selected: string;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(depth === 0);
  const isSelected = selected === node.path;
  return (
    <li>
      <div
        className="flex items-center gap-1 px-1 py-0.5 rounded cursor-pointer"
        style={{
          paddingLeft: `${depth * 12 + 4}px`,
          background: isSelected ? 'var(--accent)' : 'transparent',
          color: isSelected ? 'var(--bg)' : 'var(--text)',
        }}
        onClick={() => {
          if (node.is_dir) setOpen(!open);
          onSelect(node.path);
        }}
      >
        {node.is_dir ? (
          open ? <ChevronDown size={14} /> : <ChevronRight size={14} />
        ) : (
          <span style={{ width: 14 }} />
        )}
        {node.is_dir ? <Folder size={14} /> : <File size={14} />}
        <span className="text-sm">{node.name}</span>
        {!node.is_dir && (
          <span className="ml-auto text-xs" style={{ color: 'var(--muted)' }}>
            {formatBytes(node.size_bytes)}
          </span>
        )}
      </div>
      {node.is_dir && open && node.children.length > 0 && (
        <TreeNodeList nodes={node.children} depth={depth + 1} selected={selected} onSelect={onSelect} />
      )}
    </li>
  );
}
