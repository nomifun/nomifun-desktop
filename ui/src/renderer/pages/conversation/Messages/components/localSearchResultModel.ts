// Only the exact local-search result contract becomes clickable source UI.
// Page text remains plain text; malformed/truncated output is not an empty search.
type Source = { citationId: string; title: string; url: string; host: string; snippet: string };
export type LocalSearchResult = { query: string; sources: Source[] };

const object = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const boundedText = (value: unknown, limit: number): value is string => typeof value === 'string' && value.length <= limit;

export function localSearchPayload(text?: string): Record<string, unknown> | null {
  if (!text || text.length > 64 * 1024) return null;
  try { const value: unknown = JSON.parse(text); return object(value) ? value : null; } catch { return null; }
}

export function localSearchResult(text?: string): LocalSearchResult | null {
  const value = localSearchPayload(text);
  if (!value || !boundedText(value.query, 4096) || !object(value.provider)
    || value.provider.kind !== 'browser' || value.provider.id !== 'nomi.local.browser' || value.provider.version !== '1'
    || !Array.isArray(value.results) || value.results.length > 10) return null;
  const sources: Source[] = [];
  const ids = new Set<string>();
  for (const row of value.results) {
    if (!object(row) || !boundedText(row.title, 1024) || !row.title.trim()
      || !boundedText(row.snippet, 4096) || !boundedText(row.url, 8192)
      || typeof row.citation_id !== 'string' || !/^nomi-local-search-[a-f0-9]{32}$/.test(row.citation_id)
      || ids.has(row.citation_id) || !/^https?:\/\//i.test(row.url) || /[\\\u0000-\u0020\u007f]/.test(row.url)) return null;
    let url: URL;
    try { url = new URL(row.url); } catch { return null; }
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password) return null;
    ids.add(row.citation_id);
    sources.push({ citationId: row.citation_id, title: row.title, url: row.url, host: url.host, snippet: row.snippet });
  }
  return { query: value.query, sources };
}
