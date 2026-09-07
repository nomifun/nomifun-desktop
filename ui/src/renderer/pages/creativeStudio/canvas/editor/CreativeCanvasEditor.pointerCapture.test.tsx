/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import React from 'react';

import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
} from '../../domain';
import type { CreativeProjectRepository } from '../../services';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { testEdge, testNode, testUuid } from '../core/testFixtures';
import { canvasCommands } from '../core';
import CreativeCanvasConnectionEdge from '../product/CreativeCanvasConnectionEdge';
import CreativeCanvasEditor, { type CreativeCanvasEditorHandle } from './CreativeCanvasEditor';

afterEach(() => cleanup());

const PROJECT_ID = testUuid(910);
const node = testNode('text', 911);
const document = {
  ...createEmptyCreativeProjectDocument(PROJECT_ID),
  nodes: [node],
};
const summary = (revision = '1'): CreativeProjectSummary => ({
  projectId: PROJECT_ID,
  title: 'Pointer capture test',
  revision,
  nodeCount: 1,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 1,
});
const detail: CreativeProjectDetail = {
  project: summary(),
  document,
};
const repository: CreativeProjectRepository = {
  list: async () => [summary()],
  create: async () => summary(),
  load: async () => detail,
  save: async (_projectId, _revision, nextDocument) => ({
    ...summary('2'),
    nodeCount: nextDocument.nodes.length,
    connectionCount: nextDocument.connections.length,
  }),
  rename: async (_projectId, title) => ({ ...summary(), title }),
  remove: async () => undefined,
};

describe('CreativeCanvasEditor pointer capture', () => {
  test('multi-selects real edge hit targets and deletes them with one undoable keyboard action', async () => {
    const nodes = [testNode('image', 1201), testNode('text', 1202), testNode('video', 1203)];
    const edges = [testEdge(1211, nodes[0].id, nodes[2].id), testEdge(1212, nodes[1].id, nodes[2].id), testEdge(1213, nodes[0].id, nodes[1].id)];
    const projectId = testUuid(1200);
    const fixtureSummary = { ...summary(), projectId, nodeCount: 3, connectionCount: 3 };
    const fixtureRepository: CreativeProjectRepository = {
      ...repository,
      load: async () => ({
        project: fixtureSummary,
        document: { ...createEmptyCreativeProjectDocument(projectId), nodes, connections: edges },
      }),
    };
    const ref = React.createRef<CreativeCanvasEditorHandle>();
    const { getByRole, queryByRole } = render(withCanvasTestI18n(
      <CreativeCanvasEditor ref={ref} projectId={projectId} repository={fixtureRepository} tool='select' showSaveState={false}
        renderNode={({ node }) => <span>{node.type}</span>}
        renderEdge={(context) => <CreativeCanvasConnectionEdge {...context} ariaLabel={context.connection.id} />}
      />
    ));
    await waitFor(() => getByRole('button', { name: edges[0].id }));
    act(() => { ref.current?.dispatch(canvasCommands.setSelection([nodes[0].id])); });
    fireEvent.click(getByRole('button', { name: edges[0].id }));
    fireEvent.click(getByRole('button', { name: edges[1].id }), { shiftKey: true });
    fireEvent.click(getByRole('button', { name: edges[2].id }), { ctrlKey: true });
    fireEvent.click(getByRole('button', { name: edges[2].id }), { metaKey: true });
    expect(ref.current?.getState().selection).toMatchObject({ nodeIds: [], edgeIds: [edges[0].id, edges[1].id] });
    // Right-clicking within the selection preserves it for the batch menu.
    fireEvent.contextMenu(getByRole('button', { name: edges[1].id }));
    expect(ref.current?.getState().selection.edgeIds).toHaveLength(2);
    fireEvent.keyDown(getByRole('button', { name: edges[1].id }), { key: 'Delete' });
    expect(queryByRole('button', { name: edges[0].id })).toBeNull();
    expect(ref.current?.getState().document.connections).toEqual([edges[2]]);
    expect(ref.current?.getState().document.nodes).toEqual(nodes);
    act(() => { ref.current?.dispatch(canvasCommands.undo()); });
    expect(ref.current?.getState().document.connections).toEqual(edges);
    fireEvent.click(getByRole('button', { name: edges[0].id }));
    fireEvent.keyDown(getByRole('button', { name: edges[1].id }), { key: 'Enter', shiftKey: true });
    expect(ref.current?.getState().selection.edgeIds).toHaveLength(2);
    fireEvent.contextMenu(getByRole('button', { name: edges[2].id }));
    expect(ref.current?.getState().selection.edgeIds).toEqual([edges[2].id]);
  });

  test('connects selected nodes on a card body, saves the batch and cancels cleanly', async () => {
    const sources = [testNode('image', 1111), testNode('text', 1112)];
    const target = testNode('video', 1113);
    const projectId = testUuid(1110);
    const fixtureDocument = { ...createEmptyCreativeProjectDocument(projectId), nodes: [...sources, target] };
    const fixtureSummary = { ...summary(), projectId, nodeCount: 3 };
    const savedCounts: number[] = [];
    const fixtureRepository: CreativeProjectRepository = {
      ...repository,
      load: async () => ({ project: fixtureSummary, document: fixtureDocument }),
      save: async (_projectId, _revision, document) => {
        savedCounts.push(document.connections.length);
        return { ...fixtureSummary, revision: String(savedCounts.length + 1), connectionCount: document.connections.length };
      },
    };
    const ref = React.createRef<CreativeCanvasEditorHandle>();
    const { container, getByTestId } = render(withCanvasTestI18n(
      <CreativeCanvasEditor
        ref={ref} projectId={projectId} repository={fixtureRepository} tool='select' showSaveState={false}
        renderNode={({ node }) => <span data-testid={`node-${node.id}`}>{node.type}</span>}
        renderEdge={() => null}
      />
    ));
    await waitFor(() => getByTestId(`node-${target.id}`));
    const surface = container.querySelector<HTMLElement>('[data-canvas-surface]')!;
    const targetContent = getByTestId(`node-${target.id}`);
    const placement = targetContent.closest<HTMLElement>('[data-canvas-node-kind]')!;
    const originalElementFromPoint = window.document.elementFromPoint;
    let hitElement: Element = targetContent;
    window.document.elementFromPoint = () => hitElement;
    try {
      for (const [index, source] of sources.entries()) {
        const content = getByTestId(`node-${source.id}`);
        fireEvent.pointerDown(content, { button: 0, pointerId: 1, shiftKey: index > 0 });
        fireEvent.pointerUp(surface, { button: 0, pointerId: 1 });
      }
      expect(ref.current?.getState().selection.nodeIds).toHaveLength(2);
      const handle = getByTestId(`node-${sources[0].id}`).parentElement!.querySelector('[data-canvas-connection-handle="source"]')!;
      fireEvent.pointerDown(handle, { button: 0, pointerId: 2 });
      fireEvent.pointerMove(surface, { pointerId: 2, clientX: 400, clientY: 200 });
      expect(placement.dataset.connectionTarget).toBe('valid');
      expect(ref.current?.getState().selection.nodeIds).toHaveLength(2);
      fireEvent.pointerUp(surface, { button: 0, pointerId: 2, clientX: 400, clientY: 200 });
      expect(ref.current?.getState().document.connections.map((edge) => [edge.sourceNodeId, edge.targetNodeId])).toEqual(sources.map((source) => [source.id, target.id]));
      expect(ref.current?.getState().selection.nodeIds).toHaveLength(2);
      await act(async () => { await ref.current?.flush(); });
      expect(savedCounts.at(-1)).toBe(2);
      act(() => { ref.current?.dispatch(canvasCommands.undo()); });
      expect(ref.current?.getState().document.connections).toHaveLength(0);

      fireEvent.pointerDown(handle, { button: 0, pointerId: 3 });
      fireEvent.pointerMove(surface, { pointerId: 3, clientX: 400, clientY: 200 });
      fireEvent.keyDown(surface, { key: 'Escape' });
      fireEvent.pointerUp(surface, { button: 0, pointerId: 3 });
      expect(ref.current?.getState().document.connections).toHaveLength(0);
      expect(surface.getAttribute('data-connection-dragging')).toBeNull();
      expect(placement.getAttribute('data-connection-target')).toBeNull();

      // Reversing from an input handle also accepts the entire source card.
      hitElement = getByTestId(`node-${sources[1].id}`);
      const input = placement.querySelector('[data-canvas-connection-handle="target"]')!;
      fireEvent.pointerDown(input, { button: 0, pointerId: 4 });
      fireEvent.pointerUp(surface, { button: 0, pointerId: 4 });
      expect(ref.current?.getState().document.connections[0]).toMatchObject({ sourceNodeId: sources[1].id, targetNodeId: target.id });
    } finally {
      window.document.elementFromPoint = originalElementFromPoint;
    }
  });

  test('keeps a node double click on the node instead of retargeting it to Canvas', async () => {
    const captured = new WeakMap<HTMLElement, Set<number>>();
    const captureOwners: HTMLElement[] = [];
    const descriptors = {
      has: Object.getOwnPropertyDescriptor(
        HTMLElement.prototype,
        'hasPointerCapture'
      ),
      set: Object.getOwnPropertyDescriptor(
        HTMLElement.prototype,
        'setPointerCapture'
      ),
      release: Object.getOwnPropertyDescriptor(
        HTMLElement.prototype,
        'releasePointerCapture'
      ),
    };
    Object.defineProperties(HTMLElement.prototype, {
      hasPointerCapture: {
        configurable: true,
        value(this: HTMLElement, pointerId: number) {
          return captured.get(this)?.has(pointerId) ?? false;
        },
      },
      setPointerCapture: {
        configurable: true,
        value(this: HTMLElement, pointerId: number) {
          const pointerIds = captured.get(this) ?? new Set<number>();
          pointerIds.add(pointerId);
          captured.set(this, pointerIds);
          captureOwners.push(this);
        },
      },
      releasePointerCapture: {
        configurable: true,
        value(this: HTMLElement, pointerId: number) {
          captured.get(this)?.delete(pointerId);
        },
      },
    });

    const intents: string[] = [];
    try {
      const { getByTestId, container, unmount } = render(
        withCanvasTestI18n(
          <CreativeCanvasEditor
            projectId={PROJECT_ID}
            tool='select'
            repository={repository}
            showSaveState={false}
            renderNode={({ node: renderedNode, dragHandleProps }) => (
              <div
                data-testid={`rendered-${renderedNode.id}`}
                onPointerDown={dragHandleProps.onPointerDown}
              >
                {renderedNode.type}
              </div>
            )}
            renderEdge={() => null}
            onIntegrationIntent={(intent) => {
              intents.push(
                intent.type === 'node/open'
                  ? `${intent.type}:${intent.mode}`
                  : intent.type
              );
            }}
          />
        )
      );
      const content = await waitFor(() => getByTestId(`rendered-${node.id}`));
      const placement = content.closest<HTMLElement>('[data-canvas-node-kind]');
      const surface = container.querySelector<HTMLElement>('[data-canvas-surface]');
      if (!placement || !surface) throw new Error('Canvas fixture did not render');

      fireEvent.pointerDown(content, { button: 0, pointerId: 7 });
      expect(captureOwners).toEqual([placement]);
      expect(captureOwners.includes(surface)).toBe(false);
      fireEvent.pointerUp(placement, { button: 0, pointerId: 7 });
      fireEvent.doubleClick(
        getByTestId(`rendered-${node.id}`),
        { button: 0, clientX: 20, clientY: 20 }
      );

      await waitFor(() => {
        expect(intents.includes('node/open:edit-text')).toBe(true);
      });
      expect(intents.includes('canvas/create-node-menu/open')).toBe(false);

      intents.length = 0;
      fireEvent.doubleClick(surface, { button: 0, clientX: 80, clientY: 60 });
      await waitFor(() => {
        expect(intents.includes('canvas/create-node-menu/open')).toBe(true);
      });
      expect(intents.includes('node/open:edit-text')).toBe(false);
      unmount();
    } finally {
      for (const [name, descriptor] of [
        ['hasPointerCapture', descriptors.has],
        ['setPointerCapture', descriptors.set],
        ['releasePointerCapture', descriptors.release],
      ] as const) {
        if (descriptor) {
          Object.defineProperty(HTMLElement.prototype, name, descriptor);
        } else {
          delete (HTMLElement.prototype as unknown as Record<string, unknown>)[
            name
          ];
        }
      }
    }
  });
});
