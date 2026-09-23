import { httpRequest } from '@/common/adapter/httpBridge';

export type BrowserTarget = { tab_id: string; runtime_generation: number; document_generation: number };
export type BrowserPermission = { request_id: string; kind: string; origin: string };
export type BrowserDialog = { request_id: string; target: BrowserTarget; kind: 'alert' | 'confirm' | 'prompt' | 'before_unload'; message: string; default_text: string; origin: string; text_truncated: boolean };
export type BrowserTab = { target: BrowserTarget; title: string; url: string; lifecycle: 'loading' | 'ready' | 'failed' | 'crashed'; can_go_back: boolean; can_go_forward: boolean; zoom_percent: number; blocked_permissions?: string[]; permission_requests?: BrowserPermission[]; script_dialog?: BrowserDialog };
export type BrowserDownload = { id: string; tab_id: string; filename: string; state: 'choosing' | 'in_progress' | 'cancelling' | 'completed' | 'cancelled' | 'failed'; received_bytes: number; total_bytes: number | null; can_cancel: boolean };
export type BrowserProviderKind = 'managed' | 'attached_chrome';
export type AttachedProviderState = 'connected' | 'connection_lost' | 'disconnecting' | 'cleanup_failed';
export type AttachedProviderSnapshot = { incarnation: string; state: AttachedProviderState; chromium_major: number };
export type BrowserActionGrant = 'browser/observe' | 'browser/navigate' | 'browser/act' | 'browser/render_content' | 'browser/download' | 'browser/upload' | 'browser/evaluate';
export type BrowserSnapshot = { agent_session_id: string; resource_binding_id: string; provider_id: string; provider_kind: BrowserProviderKind; allowed_actions: BrowserActionGrant[]; run: { revision: number; input_state: 'user_ready' | 'agent_running'; input_gate_failed: boolean }; runtime: { runtime_generation: number; revision: number; active_tab_id: string | null; tabs: BrowserTab[]; downloads: BrowserDownload[] } | null };
export type BrowserCommand = { command: 'create'; url: string } | { command: 'close_all' | 'open_downloads' | 'clear_site_data'; runtime_generation: number } | { command: 'activate' | 'close' | 'back' | 'forward' | 'reload' | 'stop_loading' | 'open_external'; target: BrowserTarget } | { command: 'navigate'; target: BrowserTarget; url: string } | { command: 'set_zoom'; target: BrowserTarget; percent: number } | { command: 'permission'; target: BrowserTarget; request_id: string; allow: boolean } | { command: 'dialog'; target: BrowserTarget; request_id: string; accept: boolean; text?: string } | { command: 'cancel_download'; target: BrowserTarget; download_id: string };

export function browserCommandAction(command: BrowserCommand): BrowserActionGrant {
  switch (command.command) {
    case 'create':
    case 'activate':
    case 'set_zoom':
    case 'navigate':
    case 'back':
    case 'forward':
    case 'reload':
    case 'stop_loading':
      return 'browser/navigate';
    case 'open_downloads':
    case 'cancel_download':
      return 'browser/download';
    default:
      return 'browser/act';
  }
}
export type SurfaceBounds = { x: number; y: number; width: number; height: number };
export type BrowserShortcutAction = 'address' | 'new_tab' | 'close_tab' | 'reload' | 'back' | 'forward';
export type BrowserShortcut = { agent_session_id: string; target: BrowserTarget; action: BrowserShortcutAction };
export type BrowserViewEvent = { kind: 'snapshot'; snapshot: BrowserSnapshot } | { kind: 'unavailable'; code: string };

export function newerSnapshot(current: BrowserSnapshot | null, incoming: BrowserSnapshot): BrowserSnapshot {
  if (!current || current.agent_session_id !== incoming.agent_session_id || current.resource_binding_id !== incoming.resource_binding_id || current.provider_id !== incoming.provider_id || current.provider_kind !== incoming.provider_kind) return incoming;
  const run = incoming.run.revision < current.run.revision ? current.run : incoming.run;
  const previous = current.runtime;
  const next = incoming.runtime;
  const runtime = previous && (!next || next.runtime_generation < previous.runtime_generation || (next.runtime_generation === previous.runtime_generation && next.revision < previous.revision)) ? previous : next;
  return { ...incoming, run, runtime };
}
export interface BrowserClient {
  listenShortcuts(agentSessionId: string, listener: (shortcut: BrowserShortcut) => void): Promise<() => void>;
  closeResource(agentSessionId: string, runtimeGeneration: number): Promise<void>;
  ensure(agentSessionId: string): Promise<BrowserSnapshot>;
  attachedProvider(): Promise<AttachedProviderSnapshot | null>;
  command(agentSessionId: string, command: BrowserCommand): Promise<BrowserSnapshot>;
  attach(agentSessionId: string, bounds: SurfaceBounds, onEvent: (event: BrowserViewEvent) => void): Promise<number>;
  update(id: number, sequence: number, bounds: SurfaceBounds, visible: boolean): Promise<void>;
  detach(id: number): Promise<void>;
  scaleFactor(): Promise<number>;
}

export const browserClient: BrowserClient = {
  async listenShortcuts(id, listener) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<BrowserShortcut>('browser-capability-shortcut', event => { if (event.payload.agent_session_id === id) listener(event.payload); });
  },
  async closeResource(id, runtimeGeneration) { await httpRequest('DELETE', `/api/agent-sessions/${encodeURIComponent(id)}/browser`, { runtime_generation: runtimeGeneration }); },
  ensure: id => httpRequest('POST', `/api/agent-sessions/${encodeURIComponent(id)}/browser`, {}),
  attachedProvider: () => httpRequest('GET', '/api/browser-providers/attached-chrome'),
  command: (id, command) => httpRequest('POST', `/api/agent-sessions/${encodeURIComponent(id)}/browser/commands`, command),
  async attach(id, bounds, onEvent) {
    const { invoke, Channel } = await import('@tauri-apps/api/core');
    const events = new Channel<BrowserViewEvent>();
    events.onmessage = onEvent;
    return invoke<number>('browser_surface_attach', { agentSessionId: id, bounds, events });
  },
  async update(id, sequence, bounds, visible) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('browser_surface_update', { attachmentId: id, sequence, bounds, visible });
  },
  async detach(id) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('browser_surface_detach', { attachmentId: id });
  },
  async scaleFactor() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    return getCurrentWindow().scaleFactor();
  },
};

export function navigationUrl(text: string): string | null {
  const input = text.trim();
  if (!input) return null;
  if (/^(javascript|data|file|about):/i.test(input) || (/^[a-z][a-z0-9+.-]*:\/\//i.test(input) && !/^https?:\/\//i.test(input))) return null;
  try {
    const url = new URL(/^https?:\/\//i.test(input) ? input : /^(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(input) ? `http://${input}` : `https://${input}`);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || !url.hostname) return null;
    return url.href;
  } catch { return null; }
}
