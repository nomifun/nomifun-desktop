/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
} from '../../domain';
import type { CreativeProjectRepository } from '../../services';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { testNode, testUuid } from '../core/testFixtures';
import CreativeCanvasEditor from './CreativeCanvasEditor';

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
