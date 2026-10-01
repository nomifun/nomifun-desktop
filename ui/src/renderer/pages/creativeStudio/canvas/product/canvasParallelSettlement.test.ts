/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeTask } from '../../tasks';
import { canvasCommands, canvasReducer, createInitialCanvasState, type CanvasCommand } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import { settleCanvasImageComposeTask } from './canvasImageComposerRuntime';
import { settleCanvasVideoComposeTask } from './canvasVideoComposerRuntime';
import { settleCanvasAudioComposeTask } from './canvasAudioComposerRuntime';

const projectId = testUuid(950);
const providerId = testUuid(951);
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};
const asset = (id: string, kind: 'image' | 'video' | 'audio'): CreativeAsset => ({
  id, kind, title: id, collection: null, tags: [], mimeType: `${kind}/${kind === 'image' ? 'png' : kind === 'video' ? 'mp4' : 'wav'}`,
  width: kind === 'audio' ? null : 1024, height: kind === 'audio' ? null : 1024,
  bytes: 100, inLibrary: true, textContent: null, origin: null,
  originalUrl: `/assets/${id}`, thumbnailUrl: null, createdAt: 1, updatedAt: 1,
});

describe('parallel canvas result settlement', () => {
  test.each(['image', 'video', 'audio'] as const)(
    'recovers legacy %s tasks already sharing a source without losing results or the active selection',
    async (kind) => {
      const source = testNode(kind, 1);
      const editing = testNode('text', 2);
      const taskKind = kind === 'image' ? 'image_generation' : kind === 'video' ? 'video_generation' : 'speech_synthesis';
      const capability = kind === 'image' ? 't2i' : kind === 'video' ? 't2v' : 'tts';
      const config = (suffix: number): Extract<CreativeCanvasNode, { type: 'config' }> => {
        const base = testNode('config', suffix, { x: 400, y: suffix * 300 });
        return {
          ...base,
          data: {
            ...base.data, task: taskKind, capability, providerId, model: `${kind}-v1`,
            operation: { kind: `${kind}-node-compose`, sourceNodeId: source.id, sourceAssetId: null },
            taskId: testUuid(suffix + 960), status: 'running', inputAssetIds: [],
          },
        };
      };
      const first = config(3);
      const second = config(4);
      const task = (owner: typeof first, outputId: string): CreativeTask => ({
        taskId: owner.data.taskId!, owner: { kind: 'canvas_node', canvasId: projectId, nodeId: owner.id },
        providerId, model: `${kind}-v1`, task: taskKind, capability,
        parameters: {}, inputs: [], status: 'succeeded', error: null, resultAssetIds: [outputId],
        attempt: 1, submittedAt: 1, startedAt: 2, finishedAt: 3, deletedAt: null,
      });
      const firstAsset = asset(testUuid(970), kind);
      const secondAsset = asset(testUuid(971), kind);
      let state = createInitialCanvasState({ document: {
        nodes: [source, editing, first, second],
        connections: [first, second].map((owner, index) => ({
          id: testUuid(972 + index), sourceNodeId: source.id, targetNodeId: owner.id,
          sourceHandle: 'source', targetHandle: 'target',
        })),
      } });
      state = canvasReducer(state, canvasCommands.setSelection([editing.id]));
      let pending = [first.data.taskId!, second.data.taskId!];
      const editor = {
        getState: () => state,
        dispatch: (command: CanvasCommand) => { state = canvasReducer(state, command); return state; },
        addPendingTask: async (id: string) => { pending = [...new Set([...pending, id])]; },
        removePendingTask: async (id: string) => { pending = pending.filter((candidate) => candidate !== id); },
      };
      const results = new Map([
        [firstAsset.id, deferred<CreativeAsset>()],
        [secondAsset.id, deferred<CreativeAsset>()],
      ]);
      const assets = {
        get: async (id: string) => results.get(id)!.promise,
        list: async () => ({ items: [firstAsset, secondAsset], total: 2 }),
        upload: async () => firstAsset,
        update: async () => firstAsset,
        remove: async () => {},
        url: (id: string) => `/assets/${id}`,
      };
      const settle = (owner: typeof first, output: CreativeAsset) => {
        const input = { editor, projectId, task: task(owner, output.id), assets, viewportSize: { width: 1440, height: 900 } };
        return kind === 'image' ? settleCanvasImageComposeTask(input)
          : kind === 'video' ? settleCanvasVideoComposeTask(input)
          : settleCanvasAudioComposeTask(input);
      };
      const one = settle(first, firstAsset);
      const two = settle(second, secondAsset);
      results.get(secondAsset.id)!.resolve(secondAsset);
      await two;
      expect(pending).toEqual([first.data.taskId!]);
      expect(state.document.nodes.find((node) => node.id === source.id)?.data).toMatchObject({ assetId: secondAsset.id });
      results.get(firstAsset.id)!.resolve(firstAsset);
      await one;
      expect(pending).toEqual([]);
      expect(state.selection.nodeIds).toEqual([editing.id]);
      expect(state.selection.edgeIds).toEqual([]);
      const outputs = state.document.nodes.filter((node) => node.type === kind);
      expect(outputs).toHaveLength(2);
      const derived = outputs.find((node) => 'assetId' in node.data && node.data.assetId === firstAsset.id)!;
      expect(state.document.connections.some((edge) => edge.sourceNodeId === first.id && edge.targetNodeId === derived.id)).toBe(true);
      expect(state.document.nodes.find((node) => node.id === source.id)?.data).toMatchObject({ assetId: secondAsset.id });
      const nodeCount = state.document.nodes.length;
      const edgeCount = state.document.connections.length;
      await Promise.all([settle(first, firstAsset), settle(second, secondAsset)]);
      expect(state.document.nodes).toHaveLength(nodeCount);
      expect(state.document.connections).toHaveLength(edgeCount);
      expect(state.selection.nodeIds).toEqual([editing.id]);
    }
  );
});
