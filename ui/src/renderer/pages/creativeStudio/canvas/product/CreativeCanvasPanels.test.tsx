/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeCanvasNode } from '../../domain';
import type { CanvasState } from '../core';
import {
  CreativeCanvasAssistantUnwiredPanel,
  CreativeCanvasHistoryPanel,
  CreativeCanvasOutlinePanel,
  CreativeCanvasPropertiesPanel,
  CreativeCanvasWorkflowUnwiredPanel,
  creativeCanvasNodeDisplayName,
} from './CreativeCanvasPanels';

const groupNode: Extract<CreativeCanvasNode, { type: 'group' }> = {
    id: '0190f5fe-7c00-7a00-8000-000000000101',
    type: 'group',
    position: { x: 10, y: 20 },
    size: { width: 640, height: 480 },
    groupId: null,
    zIndex: 0,
    locked: false,
    data: { title: '第一幕素材', color: null, collapsed: false },
};

const textNode: Extract<CreativeCanvasNode, { type: 'text' }> = {
    id: '0190f5fe-7c00-7a00-8000-000000000102',
    type: 'text',
    position: { x: 80, y: 96 },
    size: { width: 280, height: 180 },
    groupId: '0190f5fe-7c00-7a00-8000-000000000101',
    zIndex: 2,
    locked: true,
    data: { text: '雨夜里的第一幕', format: 'markdown', fontSize: 18, textAlign: 'left' },
};

const nodes: CreativeCanvasNode[] = [groupNode, textNode];

const snapshot = { nodes: [], connections: [] };

const state = (overrides: Partial<CanvasState> = {}): CanvasState => ({
  document: {
    nodes,
    connections: [
      {
        id: '0190f5fe-7c00-7a00-8000-000000000103',
        sourceNodeId: nodes[0].id,
        targetNodeId: nodes[1].id,
        sourceHandle: null,
        targetHandle: null,
      },
    ],
  },
  viewport: { x: 0, y: 0, zoom: 1 },
  selection: { nodeIds: [nodes[1].id], edgeIds: [], marquee: null, box: null },
  clipboard: null,
  history: { past: [snapshot, snapshot], future: [snapshot], merge: null },
  ...overrides,
});

const noop = () => undefined;

describe('Creative Canvas product presentation panels', () => {
  test('renders a real canonical outline without inventing graph rows', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasOutlinePanel state={state()} onSelectNode={noop} onClearSelection={noop} />
    );

    expect(html.includes('data-canvas-product-panel="outline"')).toBe(true);
    expect(html.includes('2 个节点 · 1 条连接')).toBe(true);
    expect(html.includes('第一幕素材')).toBe(true);
    expect(html.includes('雨夜里的第一幕')).toBe(true);
    expect(html.includes(`data-node-id="${nodes[1].id}"`)).toBe(true);
    expect(html.includes('data-node-grouped="true"')).toBe(true);
    expect(html.includes('data-selected="true"')).toBe(true);
    expect(html.includes('节点已锁定')).toBe(true);
  });

  test('shows exact selected-node properties and canonical editable controls', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasPropertiesPanel state={state()} onUpdateNode={noop} />
    );

    expect(html.includes('data-properties-node-kind="text"')).toBe(true);
    expect(html.includes('雨夜里的第一幕')).toBe(true);
    expect(html.includes('280 × 180')).toBe(true);
    expect(html.includes('markdown')).toBe(true);
    expect(html.includes('18px')).toBe(true);
    expect(html.includes('aria-label="编辑节点属性"')).toBe(true);
    expect(html.includes('<textarea')).toBe(true);
    expect(html.includes('雨夜里的第一幕</textarea>')).toBe(true);
    expect(html.includes('锁定节点')).toBe(true);
  });

  test('renders honest empty and multi-selection property states', () => {
    const empty = renderToStaticMarkup(
      <CreativeCanvasPropertiesPanel
        state={state({ selection: { nodeIds: [], edgeIds: [], marquee: null, box: null } })}
      />
    );
    const multiple = renderToStaticMarkup(
      <CreativeCanvasPropertiesPanel
        state={state({
          selection: { nodeIds: nodes.map((node) => node.id), edgeIds: [], marquee: null, box: null },
        })}
        onSelectNode={noop}
      />
    );

    expect(empty.includes('未选择节点')).toBe(true);
    expect(multiple.includes('已选择 2 个节点')).toBe(true);
    expect((multiple.match(/<button/g) ?? []).length).toBe(2);
  });

  test('states the disconnected update boundary when no command owner is supplied', () => {
    const html = renderToStaticMarkup(<CreativeCanvasPropertiesPanel state={state()} />);
    expect(html.includes('未连接 canonical 更新命令')).toBe(true);
    expect(html.includes('aria-label="编辑节点属性"')).toBe(false);
  });

  test('history exposes only actual undo and redo snapshot counts', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasHistoryPanel state={state()} onUndo={noop} onRedo={noop} />
    );

    expect(html.includes('data-canvas-product-panel="history"')).toBe(true);
    expect(html.includes('可撤销')).toBe(true);
    expect(html.includes('>2<')).toBe(true);
    expect(html.includes('可重做')).toBe(true);
    expect(html.includes('>1<')).toBe(true);
    expect(html.includes('不会臆造历史记录')).toBe(true);
  });

  test('keeps remaining unavailable agent and workflow adapters explicit', () => {
    const html = renderToStaticMarkup(
      <>
        <CreativeCanvasAssistantUnwiredPanel />
        <CreativeCanvasWorkflowUnwiredPanel />
      </>
    );

    expect(html.includes('data-unavailable-kind="assistant"')).toBe(true);
    expect(html.includes('画布专属会话绑定')).toBe(true);
    expect(html.includes('data-unavailable-kind="workflows"')).toBe(true);
    expect(html.includes('不会显示示例模板')).toBe(true);
    expect(html.includes('<textarea')).toBe(false);
  });

  test('projects display names only from canonical node data', () => {
    expect(creativeCanvasNodeDisplayName(nodes[0])).toBe('第一幕素材');
    expect(creativeCanvasNodeDisplayName(nodes[1])).toBe('雨夜里的第一幕');
    expect(
      creativeCanvasNodeDisplayName({
        ...textNode,
        data: { ...textNode.data, text: '   ' },
      })
    ).toBe('空文本');
  });
});
