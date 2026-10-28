import { isBackendHttpError, redactSensitiveText } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';
import type { AuthPhase, FeishuActor, FeishuItem, FeishuMutation, FeishuOperation, FeishuStatus, OperationItem, RuntimeInfo } from '@/common/types/feishuConnector';

export const liveOperation = (operation: FeishuOperation) => operation.status === 'queued' || operation.status === 'running';
export const runtimeCanPrepare = (runtime: RuntimeInfo) => runtime.phase === 'missing' || (runtime.phase === 'error' && !runtime.executable);
export const runtimeCanRepair = (runtime: RuntimeInfo) => runtime.phase === 'broken' || (runtime.phase === 'error' && !!runtime.executable);
export function operationKind(operation: Pick<FeishuOperation, 'items'>): string {
  const first = operation.items[0]?.mutation.kind;
  return typeof first === 'string' && ['document_create', 'document_delete', 'base_update'].includes(first) && operation.items.every(item => item.mutation.kind === first) ? first : 'mixed';
}
export const pendingItem = (item: OperationItem) => item.status === 'unknown' || item.status === 'accepted_pending_confirmation';
export const terminalAuth = (phase: AuthPhase) => !['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(phase);
export const sameActor = (left: FeishuActor, right: FeishuActor | null | undefined) =>
  !!right && left.app_id === right.app_id && left.user_id === right.user_id;
export const hasPermission = (status: FeishuStatus | null, capability: string) =>
  status?.permissions.find(permission => permission.capability === capability)?.granted === true;
export function mutationTitle(mutation: Record<string, unknown>): string {
  return [mutation.title, mutation.document_id, mutation.record_id].find(value => typeof value === 'string') as string | undefined ?? '';
}
const writeCapabilities: Record<FeishuMutation['kind'], string> = {
  document_create: 'documents.create', document_delete: 'documents.delete', base_update: 'base.write',
};
export function scopesFor(status: FeishuStatus | null, capabilities: string[]): string[] {
  return [...new Set((status?.permissions ?? []).filter(permission => capabilities.includes(permission.capability) && !permission.granted).flatMap(permission => permission.required_scopes))];
}
export const writeScopes = (status: FeishuStatus | null, items: FeishuMutation[]) => scopesFor(status, items.map(item => writeCapabilities[item.kind]));
export const readGroups = { documents: ['documents.browse'] };
export const permissionKey = (status: FeishuStatus | null) => JSON.stringify((status?.permissions ?? []).map(item => ({ ...item, required_scopes: [...item.required_scopes].sort() })).sort((a, b) => a.capability.localeCompare(b.capability)));
export const identityKey = (status: FeishuStatus | null) => [status?.connection.id, status?.connection.profile, status?.actor?.app_id, status?.actor?.user_id].join(':');
export function errorDetails(error: unknown): Record<string, unknown> {
  if (!isBackendHttpError(error) || !error.body || typeof error.body !== 'object') return {};
  const details = (error.body as Record<string, unknown>).details;
  return details && typeof details === 'object' && !Array.isArray(details) ? details as Record<string, unknown> : {};
}
export function relatedOperation(error: unknown): string | null {
  const value = errorDetails(error).related_operation_id;
  return typeof value === 'string' && value.length > 0 ? value : null;
}
export function relatedAuthorization(error: unknown): string | null {
  const value = errorDetails(error).related_flow_id;
  return typeof value === 'string' && CANONICAL_UUID_V7.test(value) ? value : null;
}
export function feishuError(error: unknown): string {
  if (isBackendHttpError(error)) {
    const body = error.body as Record<string, unknown> | undefined;
    if (body && typeof body.error === 'string') return redactSensitiveText(body.error);
  }
  return redactSensitiveText(error instanceof Error ? error.message : String(error));
}
export function feishuResourceUrl(input: string): string {
  const value = input.trim();
  let url: URL;
  try { url = new URL(value); } catch { throw new Error('feishu.resources.invalidLink'); }
  const official = ['feishu.cn', 'larksuite.com', 'larkoffice.com'].some(domain => url.hostname === domain || url.hostname.endsWith('.' + domain));
  if (url.protocol !== 'https:' || !official || url.username || url.password || url.port || /[\\\r\n]/.test(value) || !/^\/(docx|doc|docs|sheet|sheets|base|wiki|mindnote|slides|file)\/[A-Za-z0-9_-]+\/?$/.test(url.pathname)) throw new Error('feishu.resources.invalidLink');
  return value;
}
export function documentTarget(item: FeishuItem): { document_id: string; document_type: string } | null {
  const types = ['docx', 'doc', 'sheet', 'mindnote', 'slides', 'file'];
  if (types.includes(item.kind) && /^[A-Za-z0-9_-]+$/.test(item.id)) return { document_id: item.id, document_type: item.kind };
  if (!item.url) return null;
  try {
    const [, path, id] = new URL(feishuResourceUrl(item.url)).pathname.split('/');
    const kind = path === 'docs' ? 'doc' : path === 'sheets' ? 'sheet' : path;
    return types.includes(kind) ? { document_id: id, document_type: kind } : null;
  } catch { return null; }
}

