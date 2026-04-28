import { useAuthStore } from '../store/auth'

export const MAX_UPLOAD_BYTES = 20 * 1024 * 1024

/**
 * Chat-drop attachment: uploaded to workspace + base64 kept for durable
 * re-renders post-TTL. See spec §2.1 (2026-04-19 cleanup-pass design).
 */
export interface UploadedAttachment {
  /** Path relative to user's workspace, e.g. `.attachments/<msg_id>/<filename>`. */
  workspace_path: string
  /** Base64-encoded file bytes (no data: prefix). */
  base64_data: string
  /** MIME type from the File object (e.g. `image/png`). */
  media_type: string
  /** File size in bytes. */
  size_bytes: number
  /** Original filename for display / alt text. */
  filename: string
}

/**
 * Upload a chat-drop image to the user's workspace under `.attachments/`
 * and simultaneously read it as base64. Both are returned together — the
 * workspace URL is preferred for re-renders (cacheable, small), the
 * base64 is the fallback once the 30-day TTL sweep runs.
 *
 * Errors are thrown; the caller should display them on a chip.
 */
export async function uploadChatImage(
  file: File,
  msgId: string,
  signal?: AbortSignal,
): Promise<UploadedAttachment> {
  const safeName = encodeURIComponent(file.name)
  const workspacePath = `.attachments/${msgId}/${safeName}`
  const url = `/api/workspace/files/${workspacePath}`

  const token = useAuthStore.getState().token
  if (!token) throw new Error('Not authenticated')

  const [, base64Data] = await Promise.all([
    fetch(url, {
      method: 'PUT',
      headers: {
        'Content-Type': file.type || 'application/octet-stream',
        Authorization: `Bearer ${token}`,
      },
      body: file,
      signal,
    }).then(async (r) => {
      if (!r.ok) {
        const text = await r.text().catch(() => '')
        throw new Error(`Upload failed: HTTP ${r.status}${text ? ` — ${text}` : ''}`)
      }
      // PUT returns 204 No Content; nothing to parse.
      return undefined
    }),
    fileToBase64(file),
  ])

  return {
    workspace_path: workspacePath,
    base64_data: base64Data,
    media_type: file.type || 'application/octet-stream',
    size_bytes: file.size,
    filename: file.name,
  }
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result as string
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error ?? new Error('FileReader failed'))
    reader.readAsDataURL(file)
  })
}
