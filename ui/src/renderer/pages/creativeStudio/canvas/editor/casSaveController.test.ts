/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { createEmptyCreativeProjectDocument } from '../../domain';
import { CreativeProjectRepositoryError } from '../../services';
import {
  CanvasCasSaveController,
  type CanvasSaveScheduler,
} from './casSaveController';

const PROJECT_ID = '019b0000-0000-7000-8000-000000000001';

class FakeScheduler implements CanvasSaveScheduler {
  callbacks = new Map<number, () => void>();
  delays: number[] = [];
  private nextId = 1;

  setTimeout(callback: () => void, delayMs: number): ReturnType<typeof setTimeout> {
    const id = this.nextId++;
    this.callbacks.set(id, callback);
    this.delays.push(delayMs);
    return id as unknown as ReturnType<typeof setTimeout>;
  }

  clearTimeout(timer: ReturnType<typeof setTimeout>): void {
    this.callbacks.delete(timer as unknown as number);
  }

  runLatest(): void {
    const id = Math.max(...this.callbacks.keys());
    const callback = this.callbacks.get(id);
    if (!callback) throw new Error('expected scheduled callback');
    this.callbacks.delete(id);
    callback();
  }
}

const documentWithTitle = (title: string) => ({
  ...createEmptyCreativeProjectDocument(PROJECT_ID),
  chatSessions: [
    {
      id: '019b0000-0000-7000-8000-000000000002',
      title,
      messageIds: [],
      model: null,
      pendingTurn: null,
      createdAt: 1,
      updatedAt: 1,
    },
  ],
});

describe('CanvasCasSaveController', () => {
  test('debounces edits and saves only the latest canonical document', async () => {
    const scheduler = new FakeScheduler();
    const calls: Array<{ revision: string; title: string }> = [];
    const controller = new CanvasCasSaveController(
      async (revision, document) => {
        calls.push({ revision, title: document.chatSessions[0]?.title ?? '' });
        return { revision: String(Number(revision) + 1) };
      },
      { debounceMs: 600, scheduler }
    );
    controller.reset('4', documentWithTitle('baseline'));
    controller.queue(documentWithTitle('first'));
    controller.queue(documentWithTitle('latest'));

    expect(controller.getSnapshot().status).toBe('dirty');
    expect(scheduler.delays).toEqual([600, 600]);
    expect(scheduler.callbacks.size).toBe(1);
    scheduler.runLatest();
    await controller.flush();

    expect(calls).toEqual([{ revision: '4', title: 'latest' }]);
    expect(controller.getSnapshot()).toEqual({
      status: 'saved',
      revision: '5',
      hasPendingChanges: false,
      error: null,
    });
  });

  test('serializes an edit made during an in-flight save against the returned revision', async () => {
    let finishFirst: ((revision: { revision: string }) => void) | undefined;
    const firstSave = new Promise<{ revision: string }>((resolve) => {
      finishFirst = resolve;
    });
    const calls: Array<{ revision: string; title: string }> = [];
    const controller = new CanvasCasSaveController(async (revision, document) => {
      calls.push({ revision, title: document.chatSessions[0]?.title ?? '' });
      if (calls.length === 1) return firstSave;
      return { revision: '3' };
    });
    controller.reset('1', documentWithTitle('baseline'));
    controller.queue(documentWithTitle('first'));
    const flushing = controller.flush();
    controller.queue(documentWithTitle('second'));
    finishFirst?.({ revision: '2' });
    await flushing;

    expect(calls).toEqual([
      { revision: '1', title: 'first' },
      { revision: '2', title: 'second' },
    ]);
    expect(controller.getSnapshot().revision).toBe('3');
    expect(controller.getSnapshot().hasPendingChanges).toBe(false);
  });

  test('exposes revision conflict and never retries or overwrites automatically', async () => {
    const scheduler = new FakeScheduler();
    let calls = 0;
    const controller = new CanvasCasSaveController(
      async () => {
        calls += 1;
        throw new CreativeProjectRepositoryError({
          kind: 'revision-conflict',
          message: 'remote revision changed',
          status: 409,
          backendCode: 'REVISION_CONFLICT',
        });
      },
      { scheduler }
    );
    controller.reset('8', documentWithTitle('baseline'));
    controller.queue(documentWithTitle('local edit'));
    const result = await controller.flush();

    expect(result.status).toBe('conflict');
    expect(controller.getSnapshot().status).toBe('conflict');
    controller.queue(documentWithTitle('more local edits'));
    expect(scheduler.callbacks.size).toBe(0);
    expect((await controller.flush()).status).toBe('conflict');
    expect(calls).toBe(1);
  });

  test('requires an explicit remote reset before saving after conflict', async () => {
    let conflict = true;
    const revisions: string[] = [];
    const controller = new CanvasCasSaveController(async (revision) => {
      revisions.push(revision);
      if (conflict) {
        throw new CreativeProjectRepositoryError({
          kind: 'revision-conflict',
          message: 'conflict',
        });
      }
      return { revision: '12' };
    });
    controller.reset('10', documentWithTitle('baseline'));
    controller.queue(documentWithTitle('stale'));
    await controller.flush();

    conflict = false;
    controller.reset('11', documentWithTitle('remote'));
    controller.queue(documentWithTitle('new local edit'));
    expect((await controller.flush()).status).toBe('saved');
    expect(revisions).toEqual(['10', '11']);
  });

  test('keeps a pending-task mutation blocked on conflict until explicit remote reload', async () => {
    const taskId = '019b0000-0000-7000-8000-000000000003';
    const attemptedDocuments: ReturnType<typeof createEmptyCreativeProjectDocument>[] = [];
    const controller = new CanvasCasSaveController(async (_revision, document) => {
      attemptedDocuments.push(structuredClone(document));
      throw new CreativeProjectRepositoryError({
        kind: 'revision-conflict',
        message: 'remote task feed changed',
      });
    });
    const baseline = createEmptyCreativeProjectDocument(PROJECT_ID);
    controller.reset('20', baseline);
    controller.queue({ ...baseline, pendingTaskIds: [taskId] });

    expect((await controller.flush()).status).toBe('conflict');
    expect(attemptedDocuments[0].pendingTaskIds).toEqual([taskId]);
    expect(controller.getSnapshot().hasPendingChanges).toBe(true);

    const remote = { ...baseline, pendingTaskIds: [] };
    controller.reset('21', remote);
    expect(controller.getSnapshot()).toEqual({
      status: 'idle',
      revision: '21',
      hasPendingChanges: false,
      error: null,
    });
  });

  test('retries the same durable pending-task document after a transport error', async () => {
    const taskId = '019b0000-0000-7000-8000-000000000004';
    let calls = 0;
    const controller = new CanvasCasSaveController(async (_revision, document) => {
      calls += 1;
      expect(document.pendingTaskIds).toEqual([taskId]);
      if (calls === 1) throw new Error('offline');
      return { revision: '31' };
    });
    const baseline = createEmptyCreativeProjectDocument(PROJECT_ID);
    controller.reset('30', baseline);
    controller.queue({ ...baseline, pendingTaskIds: [taskId] });

    expect((await controller.flush()).status).toBe('error');
    expect(controller.getSnapshot().hasPendingChanges).toBe(true);
    expect((await controller.flush()).status).toBe('saved');
    expect(calls).toBe(2);
    expect(controller.getSnapshot().revision).toBe('31');
  });
});
