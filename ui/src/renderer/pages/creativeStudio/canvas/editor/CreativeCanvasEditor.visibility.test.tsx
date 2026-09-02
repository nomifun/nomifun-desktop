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
  isCreativeCanvasUserNode,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
} from '../../domain';
import type { CreativeProjectRepository } from '../../services';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { canvasCommands } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import CreativeCanvasEditor, {
  type CreativeCanvasEditorHandle,
} from './CreativeCanvasEditor';

afterEach(() => cleanup());

const PROJECT_ID = testUuid(920);
const textNode = testNode('text', 921);
const configNode = testNode('config', 922);
const document = {
  ...createEmptyCreativeProjectDocument(PROJECT_ID),
  nodes: [textNode, configNode],
  connections: [
    {
      id: testUuid(923),
      sourceNodeId: textNode.id,
      targetNodeId: configNode.id,
      sourceHandle: 'source',
      targetHandle: 'target',
    },
  ],
};
const summary = (revision = '1'): CreativeProjectSummary => ({
  projectId: PROJECT_ID,
  title: 'Visibility test',
  revision,
  nodeCount: document.nodes.length,
  connectionCount: document.connections.length,
  createdAt: 1,
  updatedAt: 1,
});
const detail: CreativeProjectDetail = { project: summary(), document };
const repository: CreativeProjectRepository = {
  list: async () => [summary()],
  create: async () => summary(),
  load: async () => detail,
  save: async () => summary('2'),
  rename: async (_projectId, title) => ({ ...summary(), title }),
  remove: async () => undefined,
};

describe('CreativeCanvasEditor presentation visibility', () => {
  test('keeps internal task records canonical but outside rendering and selection', async () => {
    const editorRef = React.createRef<CreativeCanvasEditorHandle>();
    const { container, queryByTestId, getByTestId } = render(
      withCanvasTestI18n(
        <CreativeCanvasEditor
          ref={editorRef}
          projectId={PROJECT_ID}
          tool='select'
          repository={repository}
          showSaveState={false}
          isNodeVisible={isCreativeCanvasUserNode}
          renderNode={({ node }) => (
            <span data-testid={`node-${node.id}`}>{node.type}</span>
          )}
          renderEdge={({ connection }) => (
            <g data-testid={`edge-${connection.id}`} />
          )}
        />
      )
    );

    await waitFor(() => getByTestId(`node-${textNode.id}`));
    expect(queryByTestId(`node-${configNode.id}`)).toBeNull();
    expect(queryByTestId(`edge-${document.connections[0].id}`)).toBeNull();
    expect(editorRef.current?.getState().document.nodes).toHaveLength(2);

    act(() => {
      editorRef.current?.dispatch(canvasCommands.setSelection([configNode.id]));
    });
    expect(editorRef.current?.getState().selection.nodeIds).toEqual([]);

    const surface = container.querySelector<HTMLElement>('[data-canvas-surface]');
    if (!surface) throw new Error('Canvas surface did not render');
    fireEvent.keyDown(surface, { key: 'a', metaKey: true });
    expect(editorRef.current?.getState().selection.nodeIds).toEqual([
      textNode.id,
    ]);
  });
});
