// Thin fetch wrapper. Injects JWT, shapes errors, triggers logout on 401.
// Import useAuthStore lazily to avoid circular deps at module init.

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  // Lazy import avoids circular dependency at module load time
  const { useAuthStore } = await import('../store/auth')
  const token = useAuthStore.getState().token

  const headers: Record<string, string> = {}
  if (token) headers['Authorization'] = `Bearer ${token}`
  if (body !== undefined) headers['Content-Type'] = 'application/json'

  const res = await fetch(path, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })

  if (res.status === 401) {
    useAuthStore.getState().logout()
    throw new Error('Session expired — please log in again')
  }

  if (!res.ok) {
    const json = await res.json().catch(() => ({})) as Record<string, unknown>
    const errObj = json?.['error'] as Record<string, unknown> | undefined
    const msg = errObj?.['message']
    throw new Error(typeof msg === 'string' ? msg : `Request failed: HTTP ${res.status}`)
  }

  // Some endpoints return 204 No Content
  if (res.status === 204) return undefined as T

  return res.json() as Promise<T>
}

export const api = {
  get:    <T>(path: string)                => request<T>('GET',    path),
  post:   <T>(path: string, body: unknown) => request<T>('POST',   path, body),
  put:    <T>(path: string, body: unknown) => request<T>('PUT',    path, body),
  patch:  <T>(path: string, body: unknown) => request<T>('PATCH',  path, body),
  delete: <T>(path: string)                => request<T>('DELETE', path),
}
