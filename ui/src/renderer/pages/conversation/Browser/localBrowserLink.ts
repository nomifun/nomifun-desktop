/** Explicit user navigation only. This is not endpoint discovery or an egress grant. */
export type LocalBrowserLink = { url: string; mappedFrom?: string };

export function localBrowserLink(href: string): LocalBrowserLink | null {
  if (!/^https?:\/\//i.test(href) || /[\\\u0000-\u0020]/.test(href)) return null;
  try {
    const url = new URL(href);
    if (url.username || url.password) return null;
    const host = url.hostname;
    if (host === '0.0.0.0' || host === '[::]') {
      url.hostname = host === '0.0.0.0' ? '127.0.0.1' : '[::1]';
      return { url: url.href, mappedFrom: host };
    }
    if (host === 'localhost' || host === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(host)) return { url: url.href };
    return null;
  } catch { return null; }
}
