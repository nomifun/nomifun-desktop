import { isBackendHttpError, redactSensitiveText } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';
import type { AuthPhase, OfficeActor, OfficeItem, OfficeMutation, OfficeOperation, OfficeStatus, OperationItem } from '@/common/types/officeHub';

export const OFFICE_TABS = ['overview', 'documents', 'agenda', 'activity'] as const;
export type OfficeTab = typeof OFFICE_TABS[number];
export const officeTab = (value: string | null): OfficeTab => OFFICE_TABS.includes(value as OfficeTab) ? value as OfficeTab : 'overview';
export const liveOperation = (operation: OfficeOperation) => operation.status === 'queued' || operation.status === 'running';
export function operationKind(operation: Pick<OfficeOperation, 'items'>): string {
  const first = operation.items[0]?.mutation.kind;
  return typeof first === 'string' && ['task_create', 'task_complete', 'document_create', 'document_delete', 'calendar_create', 'base_update'].includes(first) && operation.items.every(item => item.mutation.kind === first) ? first : 'mixed';
}
export const pendingItem = (item: OperationItem) => item.status === 'unknown' || item.status === 'accepted_pending_confirmation';
export const terminalAuth = (phase: AuthPhase) => !['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(phase);
export const sameActor = (left: OfficeActor, right: OfficeActor | null | undefined) =>
  !!right && left.app_id === right.app_id && left.user_id === right.user_id;
export const hasPermission = (status: OfficeStatus | null, capability: string) =>
  status?.permissions.find(permission => permission.capability === capability)?.granted === true;
export function mutationTitle(mutation: Record<string, unknown>): string {
  return [mutation.title, mutation.document_id, mutation.task_id, mutation.record_id].find(value => typeof value === 'string') as string | undefined ?? '';
}
const writeCapabilities: Record<OfficeMutation['kind'], string> = {
  task_create: 'tasks.write', task_complete: 'tasks.write', document_create: 'documents.create',
  document_delete: 'documents.delete', calendar_create: 'calendar.write', base_update: 'base.write',
};
export function scopesFor(status: OfficeStatus | null, capabilities: string[]): string[] {
  return [...new Set((status?.permissions ?? []).filter(permission => capabilities.includes(permission.capability) && !permission.granted).flatMap(permission => permission.required_scopes))];
}
export const writeScopes = (status: OfficeStatus | null, items: OfficeMutation[]) => scopesFor(status, items.map(item => writeCapabilities[item.kind]));
export const readGroups = { documents: ['documents.browse'], agenda: ['calendar.read', 'tasks.read', 'tasklists.read'] };
export const localDate = () => {
  const now = new Date();
  return [now.getFullYear(), String(now.getMonth() + 1).padStart(2, '0'), String(now.getDate()).padStart(2, '0')].join('-');
};
export const localZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
export const permissionKey = (status: OfficeStatus | null) => JSON.stringify((status?.permissions ?? []).map(item => ({ ...item, required_scopes: [...item.required_scopes].sort() })).sort((a, b) => a.capability.localeCompare(b.capability)));
export const identityKey = (status: OfficeStatus | null) => [status?.connection.id, status?.connection.profile, status?.actor?.app_id, status?.actor?.user_id].join(':');
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
export function officeError(error: unknown): string {
  if (isBackendHttpError(error)) {
    const body = error.body as Record<string, unknown> | undefined;
    if (body && typeof body.error === 'string') return redactSensitiveText(body.error);
  }
  return redactSensitiveText(error instanceof Error ? error.message : String(error));
}
export function officeResourceUrl(input: string): string {
  const value = input.trim();
  let url: URL;
  try { url = new URL(value); } catch { throw new Error('office.resources.invalidLink'); }
  const official = ['feishu.cn', 'larksuite.com', 'larkoffice.com'].some(domain => url.hostname === domain || url.hostname.endsWith('.' + domain));
  if (url.protocol !== 'https:' || !official || url.username || url.password || url.port || /[\\\r\n]/.test(value) || !/^\/(docx|doc|docs|sheet|sheets|base|wiki|mindnote|slides|file)\/[A-Za-z0-9_-]+\/?$/.test(url.pathname)) throw new Error('office.resources.invalidLink');
  return value;
}
export function documentTarget(item: OfficeItem): { document_id: string; document_type: string } | null {
  const types = ['docx', 'doc', 'sheet', 'mindnote', 'slides', 'file'];
  if (types.includes(item.kind) && /^[A-Za-z0-9_-]+$/.test(item.id)) return { document_id: item.id, document_type: item.kind };
  if (!item.url) return null;
  try {
    const [, path, id] = new URL(officeResourceUrl(item.url)).pathname.split('/');
    const kind = path === 'docs' ? 'doc' : path === 'sheets' ? 'sheet' : path;
    return types.includes(kind) ? { document_id: id, document_type: kind } : null;
  } catch { return null; }
}
export function localRange(date: string): { start: string; end: string } {
  const start = new Date(date + 'T00:00:00');
  const end = new Date(start); end.setDate(end.getDate() + 1);
  return { start: start.toISOString(), end: end.toISOString() };
}
export function localEventTimes(date: string, time: string, minutes: number): { start: string; end: string } {
  const start = new Date(date + 'T' + time + ':00');
  if (!Number.isFinite(start.getTime())) throw new Error('office.validation.date');
  return { start: start.toISOString(), end: new Date(start.getTime() + minutes * 60_000).toISOString() };
}
