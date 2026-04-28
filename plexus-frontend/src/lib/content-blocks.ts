import type { ContentBlock } from './types'

/**
 * Detect and parse a stringified ContentBlock[] array coming back from the
 * REST history endpoint (GET /api/sessions/{id}/messages). The server today
 * stores user messages with attachments as a JSON-stringified
 * `Content::Blocks` value, so historical reloads arrive as a raw string that
 * the renderer would otherwise print verbatim.
 *
 * Strategy: cheap pre-check (must start with `[`), try JSON.parse, then
 * structurally validate every element matches a known block shape. If any
 * step fails, return the original string unchanged so plain text (including
 * text that happens to start with `[`) passes through untouched.
 *
 * Never throws — invalid JSON, non-arrays, unknown block types, and missing
 * required fields all fall through to the string path.
 */
export function parseContentBlocks(raw: string): ContentBlock[] | string {
  if (!raw || raw[0] !== '[') return raw
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return raw
  }
  if (!Array.isArray(parsed)) return raw
  if (parsed.length === 0) return raw
  for (const item of parsed) {
    if (!item || typeof item !== 'object') return raw
    const rec = item as Record<string, unknown>
    if (rec['type'] !== 'text' && rec['type'] !== 'image') return raw
    if (rec['type'] === 'text' && typeof rec['text'] !== 'string') return raw
    if (rec['type'] === 'image') {
      const src = rec['source']
      if (!src || typeof src !== 'object') return raw
    }
  }
  return parsed as ContentBlock[]
}
