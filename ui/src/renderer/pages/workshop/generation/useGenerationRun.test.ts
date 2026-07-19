import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { AssetId, CanvasId, CreationTaskId, ProviderId, WorkshopNodeId } from '@/common/types/ids';
import type { CreationTask, WorkshopGeneratorNodeData } from '../types';
import {
  allowSpawnAfterMountedSnapshot,
  auditMountedGenerationSnapshot,
  resolveTerminalGenerationTask,
  validateLegacyResultAssets,
} from './useGenerationRun';

const taskId = 'crt_terminal' as CreationTaskId;
const canvasId = 'wsc_canvas' as CanvasId;
const nodeId = 'wsn_card' as WorkshopNodeId;
const providerId = 'prv_provider' as ProviderId;
const asset = (suffix: string): AssetId => `wsa_${suffix}` as AssetId;

function task(patch: Partial<CreationTask> = {}): CreationTask {
  return {
    id: taskId,
    canvas_id: canvasId,
    node_id: nodeId,
    provider_id: providerId,
    model: 'model',
    capability: 't2i',
    params: {},
    status: 'succeeded',
    error: null,
    result_asset_ids: [asset('result')],
    attempt: 1,
    submitted_at: 1,
    started_at: 2,
    finished_at: 3,
    ...patch,
  };
}

function snapshot(patch: Partial<WorkshopGeneratorNodeData> = {}): WorkshopGeneratorNodeData {
  return {
    mode: 'image',
    prompt: '',
    params: {},
    mentions: [],
    status: 'success',
    taskId,
    resultAssetIds: [asset('stale')],
    ...patch,
  };
}

describe('generation snapshot reopen audit', () => {
  test('re-fetches a terminal green snapshot and exposes the backend downgrade', async () => {
    let fetched: CreationTaskId | null = null;
    const backendTask = task({
      status: 'failed',
      error: { kind: 'artifact_missing', message: 'artifact missing' },
      result_asset_ids: [],
    });

    const audit = await auditMountedGenerationSnapshot(snapshot(), {
      fetchTask: async (id) => {
        fetched = id;
        return backendTask;
      },
    });

    expect(fetched).toBe(taskId);
    expect(audit.kind).toBe('task');
    if (audit.kind !== 'task') throw new Error('expected task audit');
    const resolution = resolveTerminalGenerationTask(audit.task);
    expect(resolution.patch.status).toBe('error');
    expect(resolution.patch.resultAssetIds).toEqual([]);
    expect(resolution.patch.errorMessage).toBe('artifact missing');
  });

  test('never accepts task-less legacy success from stored ids alone', async () => {
    const ids = [asset('one'), asset('two')];
    let validated: AssetId[] = [];
    const audit = await auditMountedGenerationSnapshot(
      snapshot({ taskId: null, resultAssetIds: ids }),
      {
        validateLegacy: async (_mode, candidates) => {
          validated = candidates;
          return false;
        },
      }
    );

    expect(validated).toEqual(ids);
    expect(audit.kind).toBe('legacy-invalid');
  });

  test('requires every legacy artifact probe to pass', async () => {
    const ids = [asset('one'), asset('two'), asset('three')];
    const seen: AssetId[] = [];
    const valid = await validateLegacyResultAssets('video', ids, async (_mode, id) => {
      seen.push(id);
      return id !== ids[1];
    });

    expect(seen).toEqual(ids);
    expect(valid).toBe(false);
    expect(await validateLegacyResultAssets('image', [], async () => true)).toBe(false);
  });

  test('mount reconciliation cannot fan out the same batch on every reopen', () => {
    const source = readFileSync(new URL('./useGenerationRun.ts', import.meta.url), 'utf8');
    expect(source.includes('finalize(audit.task, { allowSpawn: false })')).toBe(true);
    expect(allowSpawnAfterMountedSnapshot('success')).toBe(false);
    expect(allowSpawnAfterMountedSnapshot('error')).toBe(false);
    expect(allowSpawnAfterMountedSnapshot('running')).toBe(true);
    expect(allowSpawnAfterMountedSnapshot('queued')).toBe(true);
  });
});
