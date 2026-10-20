import type { ToolFileSummary } from '../protocolBindings/ToolFileSummary';
export type { ToolFileSummary } from '../protocolBindings/ToolFileSummary';

const record = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const path = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && value.length <= 4096 && !/\p{Cc}/u.test(value);
const count = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

/** Accept only bounded, host-authored file metadata, never argument/result bodies. */
export function normalizeToolFileSummary(value: unknown, capability: unknown, action: unknown, confirmed: boolean): ToolFileSummary | undefined {
  if (!record(value) || capability !== 'workspace.files') return;
  const operation = value.operation;
  if (operation !== 'read' && operation !== 'write' && operation !== 'patch' && operation !== 'delete' && operation !== 'rename') return;
  if (action !== `workspace.files/${operation}` || !Array.isArray(value.files) || !value.files.length || value.files.length > 64) return;
  if (value.kind !== undefined && (value.kind !== 'instruction_scope' || operation !== 'read')) return;
  const files: ToolFileSummary['files'] = [];
  for (const file of value.files) {
    if (!record(file) || !path(file.path)) return;
    if (file.destination !== undefined && (operation !== 'rename' || !path(file.destination))) return;
    if (operation === 'rename' && !path(file.destination)) return;
    files.push({ path: file.path,
      ...(operation === 'rename' && typeof file.destination === 'string' ? { destination: file.destination } : {}),
      ...(confirmed && operation === 'patch' && count(file.added_lines) && count(file.removed_lines)
        ? { added_lines: file.added_lines, removed_lines: file.removed_lines } : {}),
    });
  }
  return { operation, files, ...(value.kind === 'instruction_scope' ? { kind: value.kind } : {}) };
}

export const withoutFileChanges = (summary: ToolFileSummary | undefined): ToolFileSummary | undefined => summary && ({
  ...summary, files: summary.files.map(file => ({ path: file.path, ...(file.destination ? { destination: file.destination } : {}) })),
});
