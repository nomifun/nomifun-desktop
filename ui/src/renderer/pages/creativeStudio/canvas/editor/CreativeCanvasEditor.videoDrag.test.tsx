/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import React from 'react';
import { SWRConfig } from 'swr';

import {
  createEmptyCreativeProjectDocument,
  parseCreativeProjectDocument,
  type CreativeProjectDocument,
  type CreativeProjectSummary,
} from '../../domain';
import type { CreativeProjectRepository } from '../../services';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { canvasCommands } from '../core';
import { testEdge, testNode, testUuid } from '../core/testFixtures';
import { CreativeNodeView } from '../nodes';
import CreativeCanvasEditor, { type CreativeCanvasEditorHandle } from './CreativeCanvasEditor';

const captured = new Map<number, HTMLElement>();
const captureMethodNames = ['hasPointerCapture', 'setPointerCapture', 'releasePointerCapture'] as const;
let captureDescriptors: Array<PropertyDescriptor | undefined>;

beforeEach(() => {
  captured.clear();
  captureDescriptors = captureMethodNames.map((name) => Object.getOwnPropertyDescriptor(HTMLElement.prototype, name));
  Object.defineProperties(HTMLElement.prototype, {
    hasPointerCapture: {
      configurable: true,
      value(this: HTMLElement, pointerId: number) { return captured.get(pointerId) === this; },
    },
    setPointerCapture: {
      configurable: true,
      value(this: HTMLElement, pointerId: number) { captured.set(pointerId, this); },
    },
    releasePointerCapture: {
      configurable: true,
      value(this: HTMLElement, pointerId: number) {
        if (captured.get(pointerId) === this) captured.delete(pointerId);
      },
    },
  });
});

afterEach(() => {
  cleanup();
  for (const [index, name] of captureMethodNames.entries()) {
    const descriptor = captureDescriptors[index];
    if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
    else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
  }
});

function createFixture(index: number, mediaState: 'resolved' | 'empty' | 'deleted' = 'resolved') {
  const projectId = testUuid(index);
  const video = testNode('video', index + 1, { x: 240, y: 180, width: 400, height: 240 });
  video.data = {
    ...video.data,
    assetId: mediaState === 'empty' ? null : testUuid(index + 5),
    posterAssetId: testUuid(index + 6),
    muted: true,
    loop: true,
    trimStartMs: 1000,
    trimEndMs: 12000,
  };
  const image = testNode('image', index + 2, { x: 20, y: 40 });
  const lockedVideo = { ...structuredClone(video), id: testUuid(index + 3), locked: true };
  const original: CreativeProjectDocument = {
    ...createEmptyCreativeProjectDocument(projectId),
    viewport: { x: 12, y: 24, zoom: 0.46 },
    nodes: [image, video, lockedVideo],
    connections: [testEdge(index + 4, image.id, video.id)],
  };
  let persisted = structuredClone(original);
  let revision = '1';
  let loadCount = 0;
  const saves: Array<{ expectedRevision: string; document: CreativeProjectDocument }> = [];
  const summary = (): CreativeProjectSummary => ({
    projectId,
    title: 'Loaded video drag',
    revision,
    nodeCount: persisted.nodes.length,
    connectionCount: persisted.connections.length,
    createdAt: 1,
    updatedAt: 1,
  });
  const repository: CreativeProjectRepository = {
    list: async () => [summary()],
    create: async () => summary(),
    load: async () => {
      loadCount += 1;
      return { project: summary(), document: structuredClone(persisted) };
    },
    save: async (savedProjectId, expectedRevision, nextDocument) => {
      expect(savedProjectId).toBe(projectId);
      expect(expectedRevision).toBe(revision);
      persisted = parseCreativeProjectDocument(structuredClone(nextDocument), projectId);
      saves.push({ expectedRevision, document: structuredClone(persisted) });
      revision = String(Number(revision) + 1);
      return summary();
    },
    rename: async (_projectId, title) => ({ ...summary(), title }),
    remove: async () => undefined,
  };
  const asset = mediaState === 'deleted'
    ? { src: '', deleted: true }
    : { src: '/test-video.mp4', posterSrc: '/test-poster.png' };
  return { projectId, video, image, lockedVideo, original, asset, repository, saves, getLoadCount: () => loadCount };
}

async function mountEditor(fixture: ReturnType<typeof createFixture>, tool: 'select' | 'pan' = 'select') {
  const ref = React.createRef<CreativeCanvasEditorHandle>();
  const intents: string[] = [];
  const view = render(withCanvasTestI18n(
    <SWRConfig value={{ provider: () => new Map() }}>
      <CreativeCanvasEditor
        ref={ref}
        projectId={fixture.projectId}
        repository={fixture.repository}
        tool={tool}
        saveDebounceMs={60000}
        showSaveState={false}
        showZoomControls={false}
        renderNode={({ node, selected, onActivate, onOpen, onToggleLock, dragHandleProps }) => (
          <CreativeNodeView
            node={node}
            selected={selected}
            placement='contained'
            asset={node.type === 'video' ? fixture.asset : undefined}
            onActivate={onActivate}
            onOpen={onOpen}
            onToggleLock={onToggleLock}
            onPointerDown={dragHandleProps.onPointerDown}
          />
        )}
        renderEdge={() => null}
        onIntegrationIntent={(intent) => { intents.push(intent.type); }}
      />
    </SWRConfig>
  ));
  await waitFor(() => expect(view.container.querySelector('[data-node-type="video"]')).not.toBeNull());
  const surface = view.container.querySelector<HTMLElement>('[data-canvas-surface]')!;
  const placement = (id = fixture.video.id) => view.container.querySelector<HTMLElement>(`[data-canvas-node-kind][data-canvas-node-id="${id}"]`)!;
  const dragSurface = (id = fixture.video.id) => {
    const target = placement(id).querySelector<HTMLElement>('[data-video-node-drag-surface]');
    if (!target) throw new Error('Loaded video must expose a Canvas drag surface');
    return target;
  };
  const node = (id = fixture.video.id) => ref.current!.getState().document.nodes.find((candidate) => candidate.id === id)!;
  return { ...view, ref, intents, surface, placement, dragSurface, node };
}

const pointer = (pointerId: number, clientX = 100, clientY = 100) => ({ pointerId, button: 0, clientX, clientY });

describe('CreativeCanvasEditor loaded video dragging', () => {
  test('moves at 46% zoom, undoes/redoes the gesture, and reloads the saved document intact', async () => {
    const fixture = createFixture(1800);
    const editor = await mountEditor(fixture);
    expect(editor.placement().querySelector('video')!.controls).toBe(false);
    fireEvent.pointerDown(editor.dragSurface(), pointer(1));
    expect(captured.get(1)).toBe(editor.placement());
    fireEvent.pointerMove(editor.placement(), pointer(1, 123, 109.2));
    fireEvent.pointerMove(editor.placement(), pointer(1, 146, 123));
    fireEvent.pointerUp(editor.placement(), pointer(1, 146, 123));
    expect(captured.has(1)).toBe(false);
    const movedPosition = { x: 340, y: 230 };
    expect(editor.node().position.x).toBeCloseTo(movedPosition.x);
    expect(editor.node().position.y).toBeCloseTo(movedPosition.y);
    expect(editor.ref.current!.getState().history.past).toHaveLength(1);

    act(() => { editor.ref.current!.dispatch(canvasCommands.undo()); });
    expect(editor.node()).toEqual(fixture.video);
    act(() => { editor.ref.current!.dispatch(canvasCommands.redo()); });
    expect(editor.node().position).toEqual(movedPosition);
    let result: Awaited<ReturnType<CreativeCanvasEditorHandle['flush']>> | undefined;
    await act(async () => { result = await editor.ref.current!.flush(); });
    expect(result?.status).toBe('saved');
    expect(fixture.saves).toHaveLength(1);
    expect(fixture.saves[0]).toEqual({
      expectedRevision: '1',
      document: {
        ...fixture.original,
        nodes: fixture.original.nodes.map((node) => node.id === fixture.video.id ? { ...node, position: movedPosition } : node),
      },
    });
    editor.unmount();
    const reloaded = await mountEditor(fixture);
    expect(fixture.getLoadCount()).toBe(2);
    expect(reloaded.node()).toEqual({ ...fixture.video, position: movedPosition });
    expect(reloaded.ref.current!.getState().document.connections).toEqual(fixture.original.connections);
    expect(reloaded.placement().querySelector('video')!.controls).toBe(false);
    expect(reloaded.ref.current!.getSaveState().hasPendingChanges).toBe(false);
  });

  test('keeps playback controls isolated while the video retains a drag surface', async () => {
    const fixture = createFixture(1820);
    const editor = await mountEditor(fixture);
    act(() => { editor.ref.current!.dispatch(canvasCommands.setSelection([fixture.video.id, fixture.image.id])); });
    const toggle = editor.placement().querySelector<HTMLButtonElement>('[data-video-node-playback-toggle]');
    if (!toggle) throw new Error('Loaded video must expose a playback toggle');
    fireEvent.pointerDown(toggle, pointer(2));
    fireEvent.pointerUp(toggle, pointer(2));
    fireEvent.click(toggle);
    const video = editor.placement().querySelector('video')!;
    fireEvent.play(video);
    expect(video.controls).toBe(false);
    expect(video.hasAttribute('disablepictureinpicture')).toBe(true);
    const controls = editor.placement().querySelector<HTMLElement>('[data-video-node-controls]')!;
    fireEvent.pointerDown(controls, pointer(3));
    fireEvent.pointerMove(controls, pointer(3, 146, 123));
    fireEvent.pointerUp(controls, pointer(3, 146, 123));
    fireEvent.doubleClick(controls);
    fireEvent.keyDown(controls, { key: 'Delete' });
    fireEvent.keyDown(controls, { key: ' ' });
    fireEvent.keyDown(toggle, { key: 'Delete' });
    expect(captured.size).toBe(0);
    expect(editor.ref.current!.getState().document).toEqual({ nodes: fixture.original.nodes, connections: fixture.original.connections });
    expect(editor.ref.current!.getState().selection.nodeIds).toEqual([fixture.video.id, fixture.image.id]);
    expect(editor.intents).toEqual([]);

    fireEvent.pointerDown(editor.dragSurface(), pointer(4));
    fireEvent.pointerMove(editor.placement(), pointer(4, 146, 123));
    fireEvent.pointerUp(editor.placement(), pointer(4, 146, 123));
    expect(editor.node().position).toEqual({ x: 340, y: 230 });
    fireEvent.click(toggle);
    fireEvent.pause(video);
    expect(editor.placement().querySelector('[data-video-node-center-play]')).not.toBeNull();
  });

  test('moves selected unlocked nodes together and leaves a locked video fixed', async () => {
    const fixture = createFixture(1840);
    const editor = await mountEditor(fixture);
    fireEvent.pointerDown(editor.placement(fixture.image.id), pointer(5));
    fireEvent.pointerUp(editor.surface, pointer(5));
    fireEvent.pointerDown(editor.dragSurface(), { ...pointer(6), shiftKey: true });
    fireEvent.pointerMove(editor.placement(), pointer(6, 146, 123));
    fireEvent.pointerUp(editor.placement(), pointer(6, 146, 123));
    expect(editor.ref.current!.getState().selection.nodeIds).toEqual([fixture.image.id, fixture.video.id]);
    expect(editor.node().position).toEqual({ x: 340, y: 230 });
    expect(editor.node(fixture.image.id).position).toEqual({ x: 120, y: 90 });

    fireEvent.pointerDown(editor.dragSurface(fixture.lockedVideo.id), pointer(7));
    fireEvent.pointerMove(editor.surface, pointer(7, 146, 123));
    fireEvent.pointerUp(editor.surface, pointer(7, 146, 123));
    expect(captured.has(7)).toBe(false);
    expect(editor.node(fixture.lockedVideo.id)).toEqual(fixture.lockedVideo);
    expect(editor.node().position).toEqual({ x: 340, y: 230 });
  });

  test('keeps empty and deleted-video placeholders draggable', async () => {
    for (const [index, mediaState] of [[1940, 'empty'], [1960, 'deleted']] as const) {
      const fixture = createFixture(index, mediaState);
      const editor = await mountEditor(fixture);
      const placeholder = editor.placement().querySelector('[data-node-empty-media]')!;
      expect(placeholder).not.toBeNull();
      expect(editor.placement().querySelector('video')).toBeNull();
      expect(editor.placement().querySelector('[data-video-node-playback-toggle]')).toBeNull();
      fireEvent.pointerDown(placeholder, pointer(11));
      fireEvent.pointerMove(editor.placement(), pointer(11, 146, 123));
      fireEvent.pointerUp(editor.placement(), pointer(11, 146, 123));
      expect(editor.node()).toEqual({ ...fixture.video, position: { x: 340, y: 230 } });
      editor.unmount();
    }
  });

  test('pans from the video in hand mode and with the middle button', async () => {
    for (const [index, tool, button] of [[1860, 'pan', 0], [1880, 'select', 1]] as const) {
      const fixture = createFixture(index);
      const editor = await mountEditor(fixture, tool);
      fireEvent.pointerDown(editor.dragSurface(), { ...pointer(8), button });
      expect(captured.get(8)).toBe(editor.surface);
      fireEvent.pointerMove(editor.surface, { ...pointer(8, 146, 123), button });
      fireEvent.pointerUp(editor.surface, { ...pointer(8, 146, 123), button });
      expect(editor.ref.current!.getState().viewport).toEqual({ x: 58, y: 47, zoom: 0.46 });
      expect(editor.node()).toEqual(fixture.video);
      expect(editor.ref.current!.getState().selection.nodeIds).toEqual([]);
      editor.unmount();
    }
  });

  test('ends canceled and lost-capture gestures without leaving a stuck drag', async () => {
    for (const [index, end] of [[1900, 'pointerCancel'], [1920, 'lostPointerCapture']] as const) {
      const fixture = createFixture(index);
      const editor = await mountEditor(fixture);
      fireEvent.pointerDown(editor.dragSurface(), pointer(9));
      fireEvent.pointerMove(editor.placement(), pointer(9, 146, 123));
      fireEvent[end](editor.placement(), pointer(9, 146, 123));
      expect(captured.size).toBe(0);
      expect(editor.ref.current!.getSaveState().hasPendingChanges).toBe(true);
      fireEvent.pointerMove(editor.surface, pointer(9, 192, 146));
      expect(editor.node().position).toEqual({ x: 340, y: 230 });
      fireEvent.pointerDown(editor.dragSurface(), pointer(10));
      fireEvent.pointerMove(editor.placement(), pointer(10, 146, 123));
      fireEvent.pointerUp(editor.placement(), pointer(10, 146, 123));
      expect(editor.node().position).toEqual({ x: 440, y: 280 });
      expect(captured.size).toBe(0);
      await act(async () => { await editor.ref.current!.flush(); });
      expect(fixture.saves.at(-1)!.document.nodes.find((node) => node.id === fixture.video.id)!.position).toEqual({ x: 440, y: 280 });
      editor.unmount();
    }
  });
});
