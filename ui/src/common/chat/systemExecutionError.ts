import { sanitizeDiagnosticDetail } from './providerDiagnostic';
import type { SystemExecutionFailure } from '../protocolBindings/SystemExecutionFailure';
export type { SystemExecutionFailure } from '../protocolBindings/SystemExecutionFailure';

/** Native owner facts from the canonical error payload, never parsed from prose. */
export function normalizeSystemExecutionFailure(value: unknown): SystemExecutionFailure | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const data = value as Record<string, unknown>;
  if (typeof data.code !== 'string' || !/^[a-z0-9_]{1,128}$/i.test(data.code)
    || typeof data.message !== 'string') return undefined;
  const kind = data.kind == null ? null : data.kind;
  const raw = data.raw_os_error == null ? null : data.raw_os_error;
  const unstarted = data.user_code_not_started == null ? null : data.user_code_not_started;
  const cleaned = data.cleanup_proven == null ? null : data.cleanup_proven;
  if ((kind !== null && (typeof kind !== 'string' || !/^[a-z]{1,64}$/i.test(kind)))
    || (raw !== null && (!Number.isInteger(raw) || Number(raw) < -2147483648 || Number(raw) > 2147483647))
    || (unstarted !== null && typeof unstarted !== 'boolean')
    || (cleaned !== null && typeof cleaned !== 'boolean')) return undefined;
  return { code: data.code, message: sanitizeDiagnosticDetail(data.message) ?? '', kind: kind as string | null,
    raw_os_error: raw as number | null, user_code_not_started: unstarted as boolean | null, cleanup_proven: cleaned as boolean | null };
}
