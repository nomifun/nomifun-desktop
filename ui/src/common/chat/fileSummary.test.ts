import { describe, expect, test } from 'bun:test';
import { mergeToolCallContent, normalizeToolCallContent, type IMessageToolCall } from './chatLib';
import { normalizeToolFileSummary, withoutFileChanges } from './fileSummary';

const patch = { operation: 'patch' as const, files: [{ path: 'src/app.ts', added_lines: 8, removed_lines: 3 }] };
const content = (overrides: Partial<IMessageToolCall['content']> = {}): IMessageToolCall['content'] => ({
  call_id: 'patch-call', name: 'apply_patch', capability_id: 'workspace.files', action_id: 'workspace.files/patch',
  status: 'completed', result_seq: 20, file_summary: patch, ...overrides,
});

describe('bounded canonical file metadata', () => {
  test('copies only the allowed wire fields and never copies file bodies or argument/result objects', () => {
    const raw = { ...patch, content: 'PRIVATE FILE BODY', output: { content: 'PRIVATE RESULT' }, arguments: { patch: 'PRIVATE PATCH' },
      files: [{ ...patch.files[0], content: 'PRIVATE FILE BODY', patch: 'PRIVATE PATCH', extra: { output: 'PRIVATE RESULT' } }] };
    const before = JSON.stringify(raw);
    expect(normalizeToolFileSummary(raw, 'workspace.files', 'workspace.files/patch', true)).toEqual(patch);
    const normalized = normalizeToolCallContent({ ...content(), file_summary: raw, arguments: raw.arguments, output: raw.output });
    expect(normalized.file_summary).toEqual(patch);
    expect(JSON.stringify(normalized)).not.toContain('PRIVATE');
    expect(JSON.stringify(raw)).toBe(before);
    expect(normalized.file_summary === raw).toBe(false);
    expect(normalized.file_summary?.files[0] === raw.files[0]).toBe(false);
  });

  test('requires the exact native capability, action and matching operation rather than a same-name tool', () => {
    for (const [capability, action] of [['external', 'workspace.files/patch'], ['workspace.files', 'external/patch'],
      ['workspace.files', 'workspace.files/read'], [undefined, 'workspace.files/patch'], ['workspace.files', undefined], [undefined, undefined]]) {
      expect(normalizeToolFileSummary(patch, capability, action, true)).toBeUndefined();
    }
    expect(normalizeToolCallContent({ ...content(), capability_id: 'external', action_id: 'external/patch' }).file_summary).toBeUndefined();
    expect(normalizeToolCallContent({ ...content(), capability_id: undefined, action_id: undefined }).file_summary).toBeUndefined();
    expect(normalizeToolFileSummary({ ...patch, operation: 'search' }, 'workspace.files', 'workspace.files/search', true)).toBeUndefined();
  });

  test('accepts 64 exact targets and rejects an empty or oversized list as a whole', () => {
    const files = Array.from({ length: 64 }, (_, index) => ({ path: `src/file ${index}.ts` }));
    expect(normalizeToolFileSummary({ operation: 'patch', files }, 'workspace.files', 'workspace.files/patch', true)?.files).toEqual(files);
    for (const invalid of [[], [...files, { path: 'extra.ts' }], null, 'src/app.ts']) {
      expect(normalizeToolFileSummary({ operation: 'patch', files: invalid }, 'workspace.files', 'workspace.files/patch', true)).toBeUndefined();
    }
  });

  test('preserves legal whitespace and exact rename targets without deriving paths from prose', () => {
    const summary = { operation: 'rename', files: [{ path: '  dir with spaces/a file.ts ', destination: ' new dir/b file.ts  ' }] };
    expect(normalizeToolFileSummary(summary, 'workspace.files', 'workspace.files/rename', true)).toEqual(summary);
    expect(normalizeToolFileSummary({ operation: 'read', files: [{ path: 'a'.repeat(4096) }] }, 'workspace.files', 'workspace.files/read', true)?.files[0].path.length).toBe(4096);
    for (const invalidPath of ['', 'a'.repeat(4097), 'dir\nfile.ts', 'dir\0file.ts', 42]) {
      expect(normalizeToolFileSummary({ operation: 'read', files: [{ path: invalidPath }] }, 'workspace.files', 'workspace.files/read', true)).toBeUndefined();
    }
    expect(normalizeToolFileSummary({ operation: 'rename', files: [{ path: 'source.ts' }] }, 'workspace.files', 'workspace.files/rename', true)).toBeUndefined();
    expect(normalizeToolFileSummary({ operation: 'write', files: [{ path: 'source.ts', destination: 'other.ts' }] }, 'workspace.files', 'workspace.files/write', true)).toBeUndefined();
  });

  test('instruction scope is read metadata rather than a claim to read every contained file', () => {
    const summary = { operation: 'read', kind: 'instruction_scope', files: [{ path: '/workspace/' }] };
    expect(normalizeToolFileSummary(summary, 'workspace.files', 'workspace.files/read', true)).toEqual(summary);
    expect(normalizeToolFileSummary({ ...summary, operation: 'write' }, 'workspace.files', 'workspace.files/write', true)).toBeUndefined();
    expect(normalizeToolFileSummary({ ...summary, kind: 'file_contents' }, 'workspace.files', 'workspace.files/read', true)).toBeUndefined();
  });

  test('line changes require a confirmed patch with both valid counts; other calls only retain targets', () => {
    expect(normalizeToolFileSummary(patch, 'workspace.files', 'workspace.files/patch', false)).toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
    for (const counts of [{ added_lines: 8 }, { removed_lines: 3 }, { added_lines: -1, removed_lines: 3 },
      { added_lines: 0.5, removed_lines: 3 }, { added_lines: Number.MAX_SAFE_INTEGER + 1, removed_lines: 3 }, { added_lines: 8, removed_lines: '3' }]) {
      expect(normalizeToolFileSummary({ operation: 'patch', files: [{ path: 'src/app.ts', ...counts }] }, 'workspace.files', 'workspace.files/patch', true))
        .toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
    }
    expect(normalizeToolFileSummary({ ...patch, operation: 'write' }, 'workspace.files', 'workspace.files/write', true))
      .toEqual({ operation: 'write', files: [{ path: 'src/app.ts' }] });
    expect(normalizeToolFileSummary({ operation: 'patch', files: [{ path: 'src/app.ts', added_lines: 0, removed_lines: 0 }] }, 'workspace.files', 'workspace.files/patch', true)?.files[0])
      .toEqual({ path: 'src/app.ts', added_lines: 0, removed_lines: 0 });
  });

  test.each([
    { status: 'error' }, { status: 'canceled' }, { status: 'running' }, { result_seq: undefined },
    { result_seq: 0 }, { runtime_hold: 'arguments_rejected' }, { error: 'Rejected patch' },
  ] satisfies Partial<IMessageToolCall['content']>[] )('unconfirmed or failed metadata drops change statistics: %j', extra => {
    expect(normalizeToolCallContent(content(extra)).file_summary).toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
  });

  test('removing change evidence keeps exact paths and rename destination without mutating the source', () => {
    const before = JSON.stringify(patch);
    expect(withoutFileChanges(patch)).toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
    expect(JSON.stringify(patch)).toBe(before);
    expect(withoutFileChanges({ operation: 'rename', files: [{ path: ' a ', destination: ' b ' }] }))
      .toEqual({ operation: 'rename', files: [{ path: ' a ', destination: ' b ' }] });
    expect(withoutFileChanges(undefined)).toBeUndefined();
  });
});

describe('file summary result corrections', () => {
  test('an older result cannot replace the exact current patch statistics', () => {
    const current = content();
    const stale = content({ result_seq: 10, file_summary: { operation: 'patch', files: [{ path: 'stale.ts', added_lines: 900, removed_lines: 900 }] } });
    const merged = mergeToolCallContent(current, stale);
    expect(merged.result_seq).toBe(20);
    expect(merged.file_summary).toEqual(patch);
    expect(current.file_summary).toEqual(patch);
  });

  test.each(['error', 'canceled'] as const)('a %s correction keeps targets but cannot retain or restore applied-line claims', status => {
    const corrected = mergeToolCallContent(content(), content({ status, result_seq: 21, file_summary: undefined, error: status === 'error' ? 'Patch not committed' : undefined }));
    expect(corrected.status).toBe(status);
    expect(corrected.result_seq).toBe(21);
    expect(corrected.file_summary).toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
    const lateSuccess = mergeToolCallContent(corrected, content({ result_seq: 20 }));
    expect(lateSuccess.status).toBe(status);
    expect(lateSuccess.result_seq).toBe(21);
    expect(lateSuccess.file_summary).toEqual({ operation: 'patch', files: [{ path: 'src/app.ts' }] });
  });
});
