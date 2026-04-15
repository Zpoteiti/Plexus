import { useAuthStore } from '../store/auth'

export interface UploadResult {
  file_id: string
  file_name: string
}

export const MAX_UPLOAD_BYTES = 20 * 1024 * 1024

export function uploadFile(
  file: File,
  onProgress: (pct: number) => void,
  signal: AbortSignal,
): Promise<UploadResult> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    const form = new FormData()
    form.append('file', file)

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) {
        onProgress(Math.round((e.loaded / e.total) * 100))
      }
    }
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText))
        } catch {
          reject(new Error('Invalid upload response'))
        }
      } else {
        reject(new Error(`Upload failed: HTTP ${xhr.status}`))
      }
    }
    xhr.onerror = () => reject(new Error('Upload network error'))
    xhr.onabort = () => reject(new Error('Upload aborted'))

    signal.addEventListener('abort', () => xhr.abort())

    xhr.open('POST', '/api/files')
    const token = useAuthStore.getState().token
    if (token) {
      xhr.setRequestHeader('Authorization', `Bearer ${token}`)
    }
    xhr.send(form)
  })
}
