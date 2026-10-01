/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { CreativeTask, CreativeTaskPort, CreateCreativeTaskInput } from '../../tasks';
import { creativeAssetClient } from '../../assets';
import { testUuid } from '../core/testFixtures';
import { CanvasGenerationRuntimeController, type CanvasGenerationRuntimeControllerOptions } from './controller';
import { prepareCanvasImageRun, prepareCanvasVideoRun } from './plans';
import type { CanvasGenerationRuntimeSnapshot, PreparedCanvasGenerationRun } from './types';

const provider: IProvider = {
  id: testUuid(900) as ProviderId,
  name: 'Generation', enabled: true, platform: 'custom',
  base_url: 'https://example.invalid', auth_scheme: 'bearer', has_credentials: true,
  models: [
    ['image-v1', 'image_generation', 'openai.images'],
    ['video-v1', 'video_generation', 'openai.videos'],
  ].map(([model, task, protocol]) => ({
    provider_id: testUuid(900) as ProviderId, model, enabled: true,
    sort_order: 0, created_at: 1, updated_at: 1,
    capabilities: [{
      task: task as 'image_generation' | 'video_generation', protocol, traits: [],
      connection_role: 'default', allow_cross_origin_credentials: false,
      provider_params: {}, created_at: 1, updated_at: 1,
    }],
  })),
};
const catalog = { status: 'ready' as const, providers: [provider], error: null };
const imagePlan = (node: number) => prepareCanvasImageRun({
  catalog, canvasId: testUuid(901), nodeId: testUuid(node),
  model: { providerId: provider.id, model: 'image-v1' },
  references: { bindings: [], assets: [] },
  operation: { task: 'image_generation', capability: 't2i' },
  prompt: `Image ${node}`, interfaceMode: 'images', quality: 'auto',
  width: 1024, height: 1024, aspectRatio: '1:1', count: 1,
});
const videoPlan = (node: number) => prepareCanvasVideoRun({
  catalog, canvasId: testUuid(901), nodeId: testUuid(node),
  model: { providerId: provider.id, model: 'video-v1' },
  references: { bindings: [], assets: [] },
  operation: { task: 'video_generation', capability: 't2v' },
  prompt: `Video ${node}`, seconds: 5, width: 1280, height: 720, taskCount: 2,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
const taskFor = (input: CreateCreativeTaskInput, status: CreativeTask['status'] = 'running'): CreativeTask => ({
  taskId: input.idempotencyKey, owner: structuredClone(input.owner),
  providerId: input.providerId, model: input.model, task: input.task, capability: input.capability,
  parameters: structuredClone(input.parameters), inputs: structuredClone(input.inputs),
  status, error: null, resultAssetIds: status === 'succeeded' ? [input.idempotencyKey] : [],
  attempt: 1, submittedAt: 1, startedAt: 2, finishedAt: status === 'running' ? null : 3,
  deletedAt: null,
});
const harness = (options: CanvasGenerationRuntimeControllerOptions = {}) => {
  const submitted: CreateCreativeTaskInput[] = [];
  const signals: AbortSignal[] = [];
  const canceled: string[] = [];
  const results = new Map<string, ReturnType<typeof deferred<CreativeTask>>>();
  const failedSubmissions = new Set<string>();
  const tasks: CreativeTaskPort = {
    async create(input, signal) {
      submitted.push(input);
      if (signal) signals.push(signal);
      if (failedSubmissions.delete(input.idempotencyKey)) throw new Error('Uncertain POST');
      results.set(input.idempotencyKey, deferred<CreativeTask>());
      return taskFor(input);
    },
    async get(reference) {
      const result = results.get(reference.taskId);
      if (!result) throw new Error(`Missing result ${reference.taskId}`);
      return result.promise;
    },
    async cancel(reference) {
      canceled.push(reference.taskId);
      const input = submitted.find((candidate) => candidate.idempotencyKey === reference.taskId)!;
      const task = taskFor(input, 'canceled');
      results.get(reference.taskId)!.resolve(task);
      return task;
    },
  };
  const controller = new CanvasGenerationRuntimeController(tasks, creativeAssetClient, options);
  const finish = (plan: PreparedCanvasGenerationRun, status: CreativeTask['status'] = 'succeeded') => {
    const input = submitted.findLast((candidate) => candidate.idempotencyKey === plan.input.idempotencyKey)!;
    results.get(input.idempotencyKey)!.resolve(taskFor(input, status));
  };
  return { controller, tasks, submitted, signals, canceled, results, failedSubmissions, finish };
};
const observe = (controller: CanvasGenerationRuntimeController, predicate: (snapshot: CanvasGenerationRuntimeSnapshot) => boolean) =>
  new Promise<void>((resolve) => {
    let unsubscribe = () => {};
    const listener = controller.subscribe((snapshot) => {
      if (predicate(snapshot)) { unsubscribe(); resolve(); }
    });
    unsubscribe = listener;
    if (predicate(controller.snapshot())) unsubscribe();
  });

describe('parallel canvas generation', () => {
  test.each(['queued', 'running'] as const)('serializes a source node across distinct config owners while its task is %s', async (status) => {
    const first = imagePlan(921);
    const second = imagePlan(922);
    const other = imagePlan(923);
    const sourceId = testUuid(924);
    const h = harness({ nodeIdForTask: (reference) => reference.owner.kind === 'canvas_node' &&
      reference.owner.nodeId === testUuid(923) ? testUuid(925) : sourceId });
    const create = h.tasks.create;
    h.tasks.create = async (input, signal) => ({ ...await create(input, signal), status });
    const one = h.controller.run(first);
    expect(h.controller.isNodeBusy(testUuid(901), sourceId)).toBe(true);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    const two = h.controller.run(other);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    expect(h.submitted).toHaveLength(2);
    expect(h.signals.every((signal) => !signal.aborted)).toBe(true);
    h.finish(first);
    await one;
    expect(h.controller.isNodeBusy(testUuid(901), sourceId)).toBe(false);
    const three = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 3);
    h.finish(second); h.finish(other);
    await Promise.all([two, three]);
  });

  test.each(['succeeded', 'failed', 'canceled'] as const)('releases the same node for its next task after %s', async (status) => {
    const h = harness();
    const first = imagePlan(926);
    const second = imagePlan(926);
    const one = h.controller.run(first);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    h.finish(first, status);
    await one;
    const two = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    h.finish(second);
    await two;
    expect(h.submitted).toHaveLength(2);
  });

  test('keeps the node busy until its terminal result has finished settlement', async () => {
    const first = imagePlan(927);
    const second = imagePlan(927);
    const settled = deferred<void>();
    const h = harness({ onSettledTask: (task) => task.taskId === first.input.idempotencyKey
      ? settled.promise : undefined });
    const one = h.controller.run(first);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    h.finish(first);
    await observe(h.controller, (snapshot) => snapshot.entries[0]?.task.status === 'succeeded');
    expect(h.controller.isNodeBusy(testUuid(901), testUuid(927))).toBe(true);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    const other = imagePlan(928);
    const two = h.controller.run(other);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    h.finish(other);
    await two;
    settled.resolve();
    await one;
    expect(h.controller.isNodeBusy(testUuid(901), testUuid(927))).toBe(false);
    const three = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 3);
    h.finish(second);
    await three;
  });

  test('an uncertain POST blocks only its source until the same-key retry or confirmed dismissal', async () => {
    const h = harness();
    const first = imagePlan(929);
    const second = imagePlan(929);
    h.failedSubmissions.add(first.input.idempotencyKey);
    await h.controller.run(first);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    const other = imagePlan(930);
    const two = h.controller.run(other);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    h.controller.dismissSubmission(0);
    const three = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    h.finish(second); h.finish(other);
    await Promise.all([two, three]);
  });

  test('recovery locks only the source node, including a failed status lookup', async () => {
    const h = harness();
    const restored = imagePlan(931);
    const second = imagePlan(931);
    const receipt = deferred<CreativeTask>();
    h.results.set(restored.input.idempotencyKey, receipt);
    const request = { reference: { ...taskFor(restored.input) }, outputKind: 'image' as const };
    const recovery = h.controller.resume([request]);
    await expect(h.controller.run(second)).rejects.toThrow('unfinished generation task');
    const other = imagePlan(932);
    const two = h.controller.run(other);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    receipt.resolve(taskFor(restored.input, 'succeeded'));
    await recovery;
    h.finish(other);
    await two;
    const missing = imagePlan(933);
    const missingRequest = { reference: { ...taskFor(missing.input) }, outputKind: 'image' as const };
    await h.controller.resume([missingRequest]);
    const next = imagePlan(933);
    await expect(h.controller.run(next)).rejects.toThrow('unfinished generation task');
    const recovered = deferred<CreativeTask>();
    recovered.resolve(taskFor(missing.input, 'failed'));
    h.results.set(missing.input.idempotencyKey, recovered);
    await h.controller.resume([missingRequest]);
    const three = h.controller.run(next);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 4);
    h.finish(next);
    await three;
  });

  test('admits consecutive runs before either completes and retains out-of-order results', async () => {
    const h = harness();
    const first = imagePlan(902);
    const second = imagePlan(903);
    const one = h.controller.run(first);
    const two = h.controller.run(second);
    expect(h.controller.snapshot().submittingCount).toBe(2);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    expect(h.controller.snapshot().entries.map((entry) => entry.order)).toEqual([0, 1]);
    expect(h.signals.every((signal) => !signal.aborted)).toBe(true);
    h.finish(second);
    await two;
    expect(h.controller.snapshot().entries.map((entry) => entry.task.status)).toEqual(['running', 'succeeded']);
    h.finish(first);
    await one;
    expect(h.controller.snapshot().entries.map((entry) => entry.outputs[0]?.assetId)).toEqual([
      first.input.idempotencyKey, second.input.idempotencyKey,
    ]);
  });

  test('assigns disjoint slots to overlapping batches and a later run', async () => {
    const h = harness();
    const first = videoPlan(904);
    const second = videoPlan(905);
    const one = h.controller.run(first);
    const two = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 4);
    expect(h.controller.snapshot().entries.map((entry) => entry.order)).toEqual([0, 1, 2, 3]);
    for (const input of h.submitted) {
      h.results.get(input.idempotencyKey)!.resolve(taskFor(input, 'succeeded'));
    }
    await Promise.all([one, two]);
    const later = imagePlan(916);
    const three = h.controller.run(later);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 5);
    expect(h.controller.snapshot().entries.map((entry) => entry.order)).toEqual([0, 1, 2, 3, 4]);
    h.finish(later);
    await three;
  });

  test('keeps uncertain submissions retryable while new tasks run', async () => {
    const h = harness();
    const first = imagePlan(906);
    const second = imagePlan(907);
    h.failedSubmissions.add(first.input.idempotencyKey);
    await h.controller.run(first);
    const two = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    expect(h.controller.snapshot().submissionFailures[0]?.input.idempotencyKey).toBe(first.input.idempotencyKey);
    const retry = h.controller.retrySubmission(0);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    expect(h.controller.snapshot().entries.map((entry) => entry.order)).toEqual([0, 1]);
    expect(h.submitted.map((input) => input.idempotencyKey)).toEqual([
      first.input.idempotencyKey, second.input.idempotencyKey, first.input.idempotencyKey,
    ]);
    h.finish(first); h.finish(second);
    await Promise.all([retry, two]);
    expect(h.controller.snapshot().submissionFailures).toEqual([]);
    expect(h.controller.snapshot().requestError).toBeNull();
  });

  test('recovery and new submissions coexist without replacing pending owners', async () => {
    const h = harness();
    const recovered = imagePlan(908);
    const restored = deferred<CreativeTask>();
    h.results.set(recovered.input.idempotencyKey, restored);
    const resume = h.controller.resume([{
      reference: { ...taskFor(recovered.input) }, outputKind: 'image',
    }]);
    const next = imagePlan(909);
    const run = h.controller.run(next);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    expect(h.controller.snapshot().recoveringCount).toBe(1);
    restored.resolve(taskFor(recovered.input, 'succeeded'));
    await resume;
    expect(h.controller.snapshot().entries.map((entry) => entry.order)).toEqual([0, 1]);
    expect(h.controller.snapshot().recoveringCount).toBe(0);
    h.finish(next);
    await run;
  });

  test('canceling one task preserves another, and cancel-all excludes future submissions', async () => {
    const h = harness();
    const first = imagePlan(910);
    const second = imagePlan(911);
    const one = h.controller.run(first);
    const two = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    await h.controller.cancel(first.input.idempotencyKey);
    await one;
    expect(h.canceled).toEqual([first.input.idempotencyKey]);
    expect(h.controller.snapshot().entries[1]?.task.status).toBe('running');
    await h.controller.cancel();
    await two;
    const third = imagePlan(912);
    const three = h.controller.run(third);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 3);
    expect(h.canceled).toEqual([first.input.idempotencyKey, second.input.idempotencyKey]);
    h.finish(third);
    await three;
    expect(h.controller.snapshot().entries[2]?.task.status).toBe('succeeded');
  });

  test('clears one confirmed-missing submission without aborting a running task', async () => {
    const h = harness();
    const missing = imagePlan(913);
    h.failedSubmissions.add(missing.input.idempotencyKey);
    await h.controller.run(missing);
    const next = imagePlan(914);
    const run = h.controller.run(next);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    h.controller.dismissSubmission(0);
    expect(h.signals.every((signal) => !signal.aborted)).toBe(true);
    expect(h.controller.snapshot().submissionFailures).toEqual([]);
    h.finish(next);
    await run;
  });

  test('rejects duplicate admission without disturbing the original worker', async () => {
    const h = harness();
    const plan = imagePlan(915);
    const run = h.controller.run(plan);
    await expect(h.controller.run(plan)).rejects.toThrow('already exists');
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    expect(h.submitted).toHaveLength(1);
    h.finish(plan);
    await run;
  });

  test('cancel-all includes pending admission but excludes tasks submitted afterward', async () => {
    const h = harness();
    const first = imagePlan(917);
    const second = imagePlan(918);
    const admission = deferred<CreativeTask>();
    const create = h.tasks.create;
    h.tasks.create = async (input, signal) => {
      const task = await create(input, signal);
      return input.idempotencyKey === first.input.idempotencyKey ? admission.promise : task;
    };
    const one = h.controller.run(first);
    const two = h.controller.run(second);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    await h.controller.cancel();
    await two;
    const third = imagePlan(919);
    const three = h.controller.run(third);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 2);
    admission.resolve(taskFor(first.input));
    await one;
    expect(h.canceled).toEqual([second.input.idempotencyKey, first.input.idempotencyKey]);
    expect(h.controller.snapshot().entries[2]?.task.status).toBe('running');
    h.finish(third);
    await three;
  });

  test('a late disposed worker cannot remove a newly resumed worker with the same task id', async () => {
    const h = harness();
    const plan = imagePlan(920);
    const late = deferred<CreativeTask>();
    const create = h.tasks.create;
    let first = true;
    h.tasks.create = async (input, signal) => {
      const task = await create(input, signal);
      if (!first) return task;
      first = false;
      return late.promise;
    };
    const old = h.controller.run(plan);
    // Let the first create reach the port before simulating StrictMode cleanup.
    await Promise.resolve();
    h.controller.dispose();
    const unsubscribe = h.controller.subscribe(() => {});
    const current = h.controller.run(plan);
    await observe(h.controller, (snapshot) => snapshot.entries.length === 1);
    late.resolve(taskFor(plan.input));
    await old;
    await expect(h.controller.run(plan)).rejects.toThrow('already exists');
    expect(h.controller.snapshot().entries[0]?.task.status).toBe('running');
    h.finish(plan);
    await current;
    expect(h.controller.snapshot().entries[0]?.task.status).toBe('succeeded');
    unsubscribe();
  });
});
