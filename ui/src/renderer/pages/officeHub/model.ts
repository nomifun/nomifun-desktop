import { isBackendHttpError, redactSensitiveText } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';
import type { AuthPhase, OfficeActor, OfficeField, OfficeMutation, OfficeOperation, OfficeStatus, OperationItem } from '@/common/types/officeHub';

export const OFFICE_TABS = ['overview', 'documents', 'agenda', 'base', 'activity'] as const;
export type OfficeTab = typeof OFFICE_TABS[number];
export const officeTab = (value: string | null): OfficeTab => OFFICE_TABS.includes(value as OfficeTab) ? value as OfficeTab : 'overview';
export const liveOperation = (operation: OfficeOperation) => operation.status === 'queued' || operation.status === 'running';
export function operationKind(operation: Pick<OfficeOperation, 'items'>): string {
  const first = operation.items[0]?.mutation.kind;
  return typeof first === 'string' && ['task_create', 'task_complete', 'document_create', 'document_save', 'calendar_create', 'base_update'].includes(first) && operation.items.every(item => item.mutation.kind === first) ? first : 'mixed';
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
  document_save: 'documents.write', calendar_create: 'calendar.write', base_update: 'base.write',
};
export function scopesFor(status: OfficeStatus | null, capabilities: string[]): string[] {
  return [...new Set((status?.permissions ?? []).filter(permission => capabilities.includes(permission.capability) && !permission.granted).flatMap(permission => permission.required_scopes))];
}
export const writeScopes = (status: OfficeStatus | null, items: OfficeMutation[]) => scopesFor(status, items.map(item => writeCapabilities[item.kind]));
export const readGroups = { documents: ['documents.search', 'documents.read'], agenda: ['calendar.read', 'tasks.read', 'tasklists.read'], base: ['base.read'] };
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
export const fieldValueText = (value: unknown) => value === null || value === undefined ? '' : typeof value === 'string' ? value : JSON.stringify(value);
export function fieldDisplayText(value: unknown): string {
  if (Array.isArray(value) && value.every(item => typeof item === 'string' || typeof item === 'number' || typeof item === 'boolean')) return value.map(String).join(', ');
  if (Array.isArray(value) && value.every(item => item && typeof item === 'object' && typeof item.text === 'string')) return value.map(item => item.text).join('');
  return fieldValueText(value);
}
export function parseBaseLink(input: string): { base_token: string; table_id: string } {
  let url: URL;
  try { url = new URL(input.trim()); } catch { throw new Error('office.base.incompleteLink'); }
  const official = ['feishu.cn', 'larksuite.com', 'larkoffice.com'].some(domain => url.hostname === domain || url.hostname.endsWith('.' + domain));
  if (url.protocol !== 'https:' || !official || url.username || url.password || url.port) throw new Error('office.base.invalidLink');
  const match = /^\/base\/([A-Za-z0-9_-]+)\/?$/.exec(url.pathname);
  const tables = url.searchParams.getAll('table');
  if (!match || tables.length !== 1 || !/^[A-Za-z0-9_-]+$/.test(tables[0])) throw new Error('office.base.incompleteLink');
  return { base_token: match[1], table_id: tables[0] };
}
export function selectFieldMode(field: OfficeField): 'single' | 'multiple' | null {
  const type = field.field_type.toLowerCase().replace(/[\s_-]/g, '');
  if (['singleselect', 'select', '3'].includes(type)) return 'single';
  if (['multiselect', 'multiplechoice', 'multipleselect', '4'].includes(type)) return 'multiple';
  return null;
}
export function selectFieldOptions(field: OfficeField): Array<{ value: string; label: string }> {
  const value = field.options;
  const options = Array.isArray(value) ? value : value && typeof value === 'object' && Array.isArray((value as Record<string, unknown>).options) ? (value as { options: unknown[] }).options : [];
  const names = options.flatMap(option => {
    if (typeof option === 'string') return [{ value: option, label: option }];
    if (!option || typeof option !== 'object') return [];
    const record = option as Record<string, unknown>;
    const name = typeof record.name === 'string' ? record.name : null;
    return name ? [{ value: name, label: name }] : [];
  });
  return names.filter((item, index) => names.findIndex(candidate => candidate.value === item.value) === index);
}
export function baseInputText(field: OfficeField, value: unknown): string {
  if (selectFieldMode(field) && selectFieldOptions(field).length) {
    return JSON.stringify(Array.isArray(value) ? value : typeof value === 'string' && value ? [value] : []);
  }
  if (['text', 'string', 'multiline_text', 'single_line_text', '1', 'number', 'currency', 'percent', '2'].includes(field.field_type.toLowerCase())) return fieldDisplayText(value);
  return fieldValueText(value);
}
export function editableFieldValue(field: OfficeField, input: string): unknown {
  const type = field.field_type.toLowerCase();
  if (['text', 'string', 'multiline_text', 'single_line_text', '1'].includes(type)) return input;
  if (['number', 'currency', 'percent', '2'].includes(type)) {
    if (!input.trim()) return null;
    const number = Number(input);
    if (!Number.isFinite(number)) throw new Error('office.validation.number');
    return number;
  }
  if (['checkbox', 'boolean', '7'].includes(type)) return input === 'true';
  try { return JSON.parse(input); } catch { throw new Error('office.validation.json'); }
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
