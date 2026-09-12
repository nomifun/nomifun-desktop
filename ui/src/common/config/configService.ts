import type { ConfigKey, ConfigKeyMap } from './configKeys';

type Subscriber = (value: unknown) => void;

declare global {
  interface Window {
    __backendPort?: number;
    __nomiLocalTrust?: string;
  }
}

function getBaseUrl(): string {
  // WebUI browser mode: no preload, fetch same-origin so web-host's
  // static-server reverse-proxies /api/* to the backend.
  if (typeof window !== 'undefined' && typeof document !== 'undefined' && !(window as Window).__backendPort) {
    return '';
  }
  const port = typeof window !== 'undefined' ? (window as Window).__backendPort || 13400 : 13400;
  return `http://127.0.0.1:${port}`;
}

/** Read the CSRF double-submit cookie (browser mode) for state-changing requests. */
function readCsrfCookie(): string | null {
  if (typeof document === 'undefined') return null;
  for (const part of document.cookie.split(';')) {
    const trimmed = part.trim();
    if (trimmed.startsWith('nomifun-csrf-token=')) {
      return decodeURIComponent(trimmed.slice('nomifun-csrf-token='.length));
    }
  }
  return null;
}

async function fetchJson<T>(method: string, path: string, body?: unknown): Promise<T> {
  const url = `${getBaseUrl()}${path}`;
  const headers: Record<string, string> = {};
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
  }
  // Desktop shell: present the per-boot local-trust secret so the backend
  // (running under TrustLocalToken) recognizes this webview as the trusted
  // local client. Without it, mutating requests (PUT) are CSRF-rejected (403).
  // Must match `httpBridge.ts` since configService bypasses that chokepoint.
  const trustSecret = typeof window !== 'undefined' ? (window as Window).__nomiLocalTrust : undefined;
  if (trustSecret) {
    headers['x-nomi-local-trust'] = trustSecret;
  } else if (
    typeof document !== 'undefined' &&
    ['POST', 'PUT', 'PATCH', 'DELETE'].includes(method.toUpperCase())
  ) {
    // WebUI browser mode (no trust secret): echo the CSRF cookie.
    const csrf = readCsrfCookie();
    if (csrf) headers['x-csrf-token'] = csrf;
  }
  const response = await fetch(url, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    cache: method.toUpperCase() === 'GET' ? 'no-store' : undefined,
  });
  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`ConfigService ${method} ${path} failed (${response.status}): ${errorBody}`);
  }
  const contentType = response.headers.get('Content-Type');
  if (!contentType?.includes('application/json')) {
    return undefined as T;
  }
  const json = await response.json();
  if (json && typeof json === 'object' && 'data' in json) {
    return json.data as T;
  }
  return json as T;
}

export class ConfigServiceImpl {
  private cache = new Map<string, unknown>();
  private subscribers = new Map<string, Set<Subscriber>>();
  private initialized = false;
  private initPromise: Promise<void> | null = null;
  // The set itself is also the identity of the current load. A reload/reset
  // invalidates older responses without cancelling callers waiting on them.
  private loadingKeys: Set<string> | null = null;
  private pendingWrites = new Set<string[]>();

  constructor(private readonly request: typeof fetchJson = fetchJson) {}

  // Idempotent: concurrent callers share the same in-flight promise, and a
  // resolved init returns immediately. Modules that need persisted settings on
  // module load (theme/colorScheme/language) await whenReady() before reading.
  //
  // IMPORTANT: this NEVER rejects. Before login (WebUI remote browser) the
  // backend returns 401/403 for /api/settings/client, and the network may be
  // unreachable. Those are expected pre-auth states, not fatal errors — the app
  // must still render (the login page!) with empty/default config. On failure we
  // resolve with an empty cache and leave `initialized = false` + clear the
  // in-flight promise so a later call (e.g. right after login via `reload()`)
  // re-fetches the authenticated settings. Local edits overlapping a load are
  // preserved on both success and failure; callers still own PUT rollback.
  initialize(): Promise<void> {
    if (this.initPromise) return this.initPromise;
    const loadingKeys = new Set([...this.pendingWrites].flat());
    this.loadingKeys = loadingKeys;
    // Publish initPromise before invoking the transport, including a transport
    // that throws synchronously. A reset before dispatch also skips the GET.
    this.initPromise = Promise.resolve().then(async () => {
      if (this.loadingKeys !== loadingKeys) return;
      try {
        const data = await this.request<Record<string, unknown>>('GET', '/api/settings/client');
        if (this.loadingKeys !== loadingKeys) return;
        this.initialized = true;
        this.loadingKeys = null;
        this.replaceCache(data ?? {}, loadingKeys);
      } catch (error) {
        if (this.loadingKeys !== loadingKeys) return;
        console.warn('[configService] settings unavailable (pre-login or offline); using empty config:', error);
        this.initialized = false;
        this.initPromise = null;
        this.loadingKeys = null;
        this.replaceCache({}, loadingKeys);
      }
    });
    return this.initPromise;
  }

  /** Force a re-fetch of settings — call right after login, when the backend
   *  starts returning the authenticated client settings. */
  async reload(): Promise<void> {
    this.initPromise = null;
    this.initialized = false;
    await this.initialize();
  }

  whenReady(): Promise<void> {
    return this.initialize();
  }

  get<K extends ConfigKey>(key: K): ConfigKeyMap[K] | undefined {
    return this.cache.get(key) as ConfigKeyMap[K] | undefined;
  }

  async set<K extends ConfigKey>(key: K, value: ConfigKeyMap[K]): Promise<void> {
    await this.write({ [key]: value });
  }

  setLocal<K extends ConfigKey>(key: K, value: ConfigKeyMap[K]): void {
    this.loadingKeys?.add(key);
    this.cache.set(key, value);
    this.notify(key, value);
  }

  async remove(key: ConfigKey): Promise<void> {
    await this.write({ [key]: null });
  }

  async setBatch(entries: Partial<{ [K in ConfigKey]: ConfigKeyMap[K] }>): Promise<void> {
    await this.write(entries);
  }

  subscribe(key: ConfigKey, callback: Subscriber): () => void {
    if (!this.subscribers.has(key)) {
      this.subscribers.set(key, new Set());
    }
    const subscribers = this.subscribers.get(key)!;
    subscribers.add(callback);
    return () => {
      subscribers.delete(callback);
    };
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  reset(): void {
    this.cache.clear();
    this.subscribers.clear();
    this.initialized = false;
    this.initPromise = null;
    this.loadingKeys = null;
    this.pendingWrites.clear();
  }

  private replaceCache(data: Record<string, unknown>, loadingKeys: Set<string>): void {
    const previous = this.cache;
    const next = new Map(Object.entries(data));
    for (const key of loadingKeys) {
      if (previous.has(key)) next.set(key, previous.get(key));
      else next.delete(key); // Preserve removals as well as optimistic values.
    }
    this.cache = next;
    // A successful load also resolves absent preferences: consumers may have
    // used a startup hint while the server settings were unavailable.
    for (const key of new Set([...previous.keys(), ...next.keys(), ...this.subscribers.keys()])) {
      if (!Object.is(previous.get(key), this.cache.get(key)) || (this.initialized && !this.cache.has(key))) {
        this.notify(key as ConfigKey, this.cache.get(key));
      }
    }
  }

  private async write(entries: Record<string, unknown>): Promise<void> {
    const keys = Object.keys(entries);
    // One identity per request: completion of an older PUT (even after reset)
    // cannot untrack a newer PUT touching the same key.
    this.pendingWrites.add(keys);
    try {
      for (const [key, value] of Object.entries(entries)) {
        this.loadingKeys?.add(key);
        if (value === null) this.cache.delete(key);
        else this.cache.set(key, value);
      }
      // Publish the whole batch before any subscriber reads related keys.
      for (const key of keys) this.notify(key as ConfigKey, this.cache.get(key));
      await this.request<void>('PUT', '/api/settings/client', entries);
    } finally {
      this.pendingWrites.delete(keys);
    }
  }

  private notify(key: ConfigKey, value: unknown): void {
    const subs = this.subscribers.get(key);
    if (subs) {
      for (const cb of [...subs]) {
        try {
          cb(value);
        } catch (error) {
          console.error('[configService] subscriber failed:', error);
        }
      }
    }
  }
}

export const configService = new ConfigServiceImpl();
