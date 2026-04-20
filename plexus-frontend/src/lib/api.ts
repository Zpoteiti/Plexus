// Thin fetch wrapper. Injects JWT, shapes errors, triggers logout on 401.
// Import useAuthStore lazily to avoid circular deps at module init.

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

// Cache the store after first import to avoid per-request microtask yield
let _authStore: typeof import('../store/auth')['useAuthStore'] | null = null
async function getAuthStore() {
  return _authStore ??= (await import('../store/auth')).useAuthStore
}

// Server returns { error: { code, message } }. For VALIDATION_FAILED the
// message is a structured string like "field errors: foo=reason; bar=reason".
// ApiError preserves both so callers that care can parse field errors.
export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  const useAuthStore = await getAuthStore()
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
    throw new ApiError(401, 'AUTH_FAILED', 'Session expired — please log in again')
  }

  if (!res.ok) {
    const json = await res.json().catch(() => ({})) as Record<string, unknown>
    const errObj = json?.['error'] as Record<string, unknown> | undefined
    const code = typeof errObj?.['code'] === 'string' ? errObj['code'] as string : 'UNKNOWN'
    const msgRaw = errObj?.['message']
    const msg = typeof msgRaw === 'string' ? msgRaw : `Request failed: HTTP ${res.status}`
    throw new ApiError(res.status, code, msg)
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
  delete: <T>(path: string, body?: unknown) => request<T>('DELETE', path, body),
}
