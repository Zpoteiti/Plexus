import { useEffect, useState, type ChangeEvent } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeft, ChevronRight, ChevronDown, File, Folder } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import { api } from '../lib/api';
import type { WorkspaceFile, WorkspaceQuota } from '../lib/types';
import { ConfirmModal } from '../components/ConfirmModal';

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
          className="w-1/4 overflow-y-auto border-r p-2 flex flex-col gap-2"
          style={{ borderColor: 'var(--border)', background: 'var(--sidebar)' }}
        >
          <div className="flex flex-col gap-1">
            <div className="text-xs uppercase font-semibold" style={{ color: 'var(--muted)' }}>
              Quick access
            </div>
            {[
              { name: 'Soul', path: 'SOUL.md' },
              { name: 'Memory', path: 'MEMORY.md' },
              { name: 'Heartbeat Tasks', path: 'HEARTBEAT.md' },
            ].map((q) => (
              <button
                key={q.path}
                onClick={() => setParams({ path: q.path })}
                className="text-left text-sm px-1 py-0.5 rounded hover:opacity-70"
                style={{
                  background: selectedPath === q.path ? 'var(--accent)' : 'transparent',
                  color: selectedPath === q.path ? 'var(--bg)' : 'var(--text)',
                }}
              >
                📄 {q.name}
              </button>
            ))}
          </div>
          <hr style={{ borderColor: 'var(--border)' }} />
          {tree ? (
            <TreeView entries={tree} selected={selectedPath} onSelect={(path) => setParams({ path })} />
          ) : (
            <div style={{ color: 'var(--muted)' }}>Loading…</div>
          )}
        </aside>

        <section className="flex-1 overflow-y-auto p-4">
          <ContentPane
            path={selectedPath}
            onChanged={(info) => {
              void refresh();
              if (info?.deleted) setParams({});
            }}
          />
        </section>
      </main>
      <UploadDropZone onUploaded={() => void refresh()} />
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

function ContentPane({
  path,
  onChanged,
}: {
  path: string;
  onChanged: (info?: { deleted?: boolean }) => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [bytes, setBytes] = useState<ArrayBuffer | null>(null);
  const [blobUrl, setBlobUrl] = useState<string | null>(null);
  const [mime, setMime] = useState<string>('');
  const [editBuf, setEditBuf] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    if (!path) {
      setText(null);
      setBytes(null);
      if (blobUrl) URL.revokeObjectURL(blobUrl);
      setBlobUrl(null);
      setMime('');
      setEditBuf(null);
      return;
    }
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  // Revoke blob URL on unmount.
  useEffect(() => {
    return () => {
      if (blobUrl) URL.revokeObjectURL(blobUrl);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`/api/workspace/file?path=${encodeURIComponent(path)}`, {
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      const buf = await res.arrayBuffer();
      const mimeType = res.headers.get('Content-Type') ?? '';
      setBytes(buf);
      setMime(mimeType);

      if (mimeType.startsWith('text/') || mimeType === 'application/json') {
        setText(new TextDecoder().decode(buf));
      } else {
        setText(null);
      }

      // Revoke any prior blob URL before creating a new one.
      if (blobUrl) URL.revokeObjectURL(blobUrl);
      if (mimeType.startsWith('image/')) {
        const url = URL.createObjectURL(new Blob([buf], { type: mimeType }));
        setBlobUrl(url);
      } else {
        setBlobUrl(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'load failed');
    } finally {
      setLoading(false);
    }
  }

  async function save() {
    if (editBuf === null) return;
    setSaving(true);
    setError(null);
    try {
      const res = await fetch(`/api/workspace/file?path=${encodeURIComponent(path)}`, {
        method: 'PUT',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
        body: editBuf,
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      setText(editBuf);
      setEditBuf(null);
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'save failed');
    } finally {
      setSaving(false);
    }
  }

  async function doDelete() {
    setDeleting(true);
    try {
      const url = `/api/workspace/file?path=${encodeURIComponent(path)}&recursive=true`;
      const res = await fetch(url, {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      setConfirmDelete(false);
      onChanged({ deleted: true });
    } catch (e) {
      setError(e instanceof Error ? e.message : 'delete failed');
    } finally {
      setDeleting(false);
    }
  }

  if (!path) return <div style={{ color: 'var(--muted)' }}>Select a file to view its contents.</div>;
  if (loading) return <div style={{ color: 'var(--muted)' }}>Loading…</div>;
  if (error) return <div style={{ color: '#ef4444' }}>{error}</div>;

  const isEditable = text !== null;
  const inEditMode = editBuf !== null;
  const isMarkdown = path.toLowerCase().endsWith('.md');
  const filename = path.split('/').pop() ?? path;

  return (
    <div className="flex flex-col gap-4 h-full">
      <div className="flex items-center gap-2">
        <div
          className="text-xs px-2 py-1 rounded flex-1"
          style={{ background: 'var(--sidebar)', color: 'var(--muted)' }}
        >
          {path}
        </div>
        {isEditable && !inEditMode && (
          <button
            onClick={() => setEditBuf(text)}
            className="text-xs px-2 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Edit
          </button>
        )}
        {!inEditMode && (
          <button
            onClick={() => setConfirmDelete(true)}
            className="text-xs px-2 py-1 rounded"
            style={{ border: '1px solid var(--border)', color: '#ef4444' }}
          >
            Delete
          </button>
        )}
        {inEditMode && (
          <>
            <button
              onClick={save}
              disabled={saving}
              className="text-xs px-2 py-1 rounded"
              style={{ background: 'var(--accent)', color: 'var(--bg)' }}
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
            <button
              onClick={() => setEditBuf(null)}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Cancel
            </button>
          </>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {inEditMode ? (
          <textarea
            value={editBuf ?? ''}
            onChange={(e) => setEditBuf(e.target.value)}
            className="w-full h-full text-sm font-mono p-4"
            style={{
              background: 'var(--card)',
              color: 'var(--text)',
              border: '1px solid var(--border)',
            }}
          />
        ) : isMarkdown && text !== null ? (
          <div className="prose prose-invert max-w-none">
            <ReactMarkdown>{text}</ReactMarkdown>
          </div>
        ) : text !== null ? (
          <pre
            className="text-sm whitespace-pre-wrap"
            style={{ background: 'var(--card)', padding: '1rem', borderRadius: '4px' }}
          >
            {text}
          </pre>
        ) : mime.startsWith('image/') && blobUrl ? (
          <div className="flex flex-col gap-2 items-start">
            <img
              src={blobUrl}
              alt={filename}
              className="max-w-full max-h-full object-contain"
            />
            <a
              href={blobUrl}
              download={filename}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Download
            </a>
          </div>
        ) : (
          <div className="flex flex-col gap-2 items-start">
            <div style={{ color: 'var(--muted)' }}>
              Binary file — {bytes ? formatBytes(bytes.byteLength) : '?'} · {mime || 'application/octet-stream'}
            </div>
            <button
              onClick={() => {
                if (!bytes) return;
                const url = URL.createObjectURL(new Blob([bytes], { type: mime || 'application/octet-stream' }));
                const a = document.createElement('a');
                a.href = url;
                a.download = filename;
                document.body.appendChild(a);
                a.click();
                document.body.removeChild(a);
                URL.revokeObjectURL(url);
              }}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Download
            </button>
          </div>
        )}
      </div>
      <ConfirmModal
        open={confirmDelete}
        title={`Delete ${path}?`}
        message="This cannot be undone. If this is a directory, its contents will be removed recursively."
        confirmLabel={deleting ? 'Deleting…' : 'Delete'}
        destructive
        onConfirm={doDelete}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  );
}

function UploadDropZone({ onUploaded }: { onUploaded: () => void }) {
  const [dragActive, setDragActive] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const enter = (e: DragEvent) => { e.preventDefault(); setDragActive(true); };
    const over = (e: DragEvent) => { e.preventDefault(); };
    const leave = (e: DragEvent) => {
      // Only reset when leaving the window entirely.
      if ((e.target as HTMLElement).nodeName === 'HTML') setDragActive(false);
    };
    const drop = (e: DragEvent) => {
      e.preventDefault();
      setDragActive(false);
      if (e.dataTransfer?.files) void uploadFiles(Array.from(e.dataTransfer.files));
    };
    window.addEventListener('dragenter', enter);
    window.addEventListener('dragover', over);
    window.addEventListener('dragleave', leave);
    window.addEventListener('drop', drop);
    return () => {
      window.removeEventListener('dragenter', enter);
      window.removeEventListener('dragover', over);
      window.removeEventListener('dragleave', leave);
      window.removeEventListener('drop', drop);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function uploadFiles(files: File[]) {
    if (files.length === 0) return;
    setUploading(true);
    setError(null);
    try {
      const form = new FormData();
      for (const f of files) form.append('files', f, f.name);
      const res = await fetch('/api/workspace/upload', {
        method: 'POST',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
        body: form,
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      onUploaded();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'upload failed');
    } finally {
      setUploading(false);
    }
  }

  function onPick(e: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(e.target.files ?? []);
    void uploadFiles(files);
    e.target.value = '';
  }

  return (
    <>
      <div
        className="border-t px-4 py-2 flex items-center gap-2"
        style={{ borderColor: 'var(--border)' }}
      >
        <label
          className="text-xs px-2 py-1 rounded cursor-pointer"
          style={{ border: '1px solid var(--border)' }}
        >
          {uploading ? 'Uploading…' : 'Upload'}
          <input type="file" multiple onChange={onPick} className="hidden" />
        </label>
        <span className="text-xs" style={{ color: 'var(--muted)' }}>
          or drop files anywhere on this page
        </span>
        {error && <span className="text-xs" style={{ color: '#ef4444' }}>{error}</span>}
      </div>
      {dragActive && (
        <div
          className="fixed inset-0 pointer-events-none z-50"
          style={{
            border: '3px dashed var(--accent)',
            background: 'rgba(57, 255, 20, 0.1)',
          }}
        />
      )}
    </>
  );
}
