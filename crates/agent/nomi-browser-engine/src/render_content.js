(() => {
  if (document.readyState !== 'complete' || !document.documentElement) return { state: 'waiting' };
  const now = performance.now();
  let state = globalThis.__nomifunRenderedContent;
  if (!state || state.document !== document) {
    state = { document, start: now, last: now };
    new MutationObserver(() => { state.last = performance.now(); }).observe(document.documentElement,
      { subtree: true, childList: true, characterData: true, attributes: true });
    globalThis.__nomifunRenderedContent = state;
  }
  // A snapshot, not a promise that arbitrary page JS has finished forever.
  if (now - state.start < 2000 && (now - state.start < 400 || now - state.last < 250)) return { state: 'waiting' };
  const bytes = new TextEncoder().encode(document.documentElement.outerHTML);
  let end = Math.min(bytes.length, 256 * 1024);
  while (end > 0 && end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
  return { state: 'ready', final_url: location.href,
    html: new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(0, end)),
    html_truncated: end < bytes.length };
})()
