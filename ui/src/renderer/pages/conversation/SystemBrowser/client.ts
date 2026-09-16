import { httpRequest } from '@/common/adapter/httpBridge';

export type SystemBrowserState = 'connecting' | 'connected' | 'connection_lost' | 'disconnecting' | 'disconnected' | 'cleanup_failed';
export type SystemBrowserTab = { tab_id: string; title: string; url: string };
export type SystemBrowserSnapshot = { incarnation: string; state: SystemBrowserState; tabs: SystemBrowserTab[] };
export type SystemBrowserChoice = { choice_id: string; title: string; url: string };
export interface SystemBrowserClient {
  snapshot(id: string): Promise<SystemBrowserSnapshot | null>;
  connect(id: string, expectedIncarnation?: string): Promise<SystemBrowserSnapshot>;
  choices(id: string, incarnation: string): Promise<{ tabs: SystemBrowserChoice[] }>;
  grant(id: string, incarnation: string, choiceId: string): Promise<SystemBrowserSnapshot>;
  disconnect(id: string, incarnation: string): Promise<SystemBrowserSnapshot>;
}
const path = (id: string) => `/api/conversations/${encodeURIComponent(id)}/system-browser`;
// httpRequest owns its timeout AbortController; it has no caller AbortSignal.
// Mutations are never retried here. Their uncertain results are reconciled by GET.
const options = { timeoutMs: 30_000 };
export const systemBrowserClient: SystemBrowserClient = {
  snapshot: id => httpRequest('GET', path(id), undefined, options),
  connect: (id, expectedIncarnation) => httpRequest('POST', path(id), expectedIncarnation ? { expected_incarnation: expectedIncarnation } : {}, options),
  choices: (id, incarnation) => httpRequest('POST', `${path(id)}/choices`, { incarnation }, options),
  grant: (id, incarnation, choiceId) => httpRequest('POST', `${path(id)}/tabs`, { incarnation, choice_id: choiceId }, options),
  disconnect: (id, incarnation) => httpRequest('DELETE', path(id), { incarnation }, options),
};
