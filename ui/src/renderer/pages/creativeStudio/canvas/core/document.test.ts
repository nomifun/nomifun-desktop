/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  createEmptyCreativeProjectDocument,
  parseCreativeProjectDocument,
} from '../../domain';
import {
  copyCanvasFragment,
  getCanvasGroups,
  groupCanvasNodes,
  materializeCanvasPaste,
  ungroupCanvasNodes,
} from './document';
import {
  sequentialTestIdFactory,
  testDocument,
  testEdge,
  testNode,
  testUuid,
} from './testFixtures';

describe('Creative Studio grouping', () => {
  test('creates one canonical group node around two free nodes', () => {
    const left = testNode('text', 1, { x: 100, y: 80, width: 120, height: 60, zIndex: 4 });
    const right = testNode('image', 2, { x: 300, y: 160, width: 200, height: 100, zIndex: 8 });
    const result = groupCanvasNodes(testDocument([left, right]), [left.id, right.id], {
      groupId: testUuid(3),
      title: 'Storyboard',
      padding: 24,
    });

    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect('groups' in result.document).toBe(false);
    expect('edges' in result.document).toBe(false);
    expect(result.document.connections).toEqual([]);
    expect(result.group).toEqual({
      id: testUuid(3),
      type: 'group',
      position: { x: 76, y: 56 },
      size: { width: 448, height: 228 },
      groupId: null,
      zIndex: 3,
      locked: false,
      data: { title: 'Storyboard', color: null, collapsed: false },
    });
    expect(getCanvasGroups(result.document)).toEqual([result.group]);
    expect(
      result.document.nodes
        .filter((node) => node.type !== 'group')
        .map((node) => node.groupId)
    ).toEqual([result.group.id, result.group.id]);

    const persisted = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(testUuid(99)),
      nodes: result.document.nodes,
      connections: result.document.connections,
    });
    expect(persisted.nodes.find((node) => node.id === result.group.id)?.type).toBe('group');
  });

  test('rejects one-node and nested group requests', () => {
    const free = testNode('text', 1);
    const grouped = testNode('image', 2, { groupId: testUuid(9) });
    expect(groupCanvasNodes(testDocument([free]), [free.id])).toEqual({
      ok: false,
      reason: 'requires_two_nodes',
    });
    expect(groupCanvasNodes(testDocument([free, grouped]), [free.id, grouped.id])).toEqual({
      ok: false,
      reason: 'nested_group_not_supported',
    });
  });

  test('ungroups without changing absolute member positions', () => {
    const group = testNode('group', 3, { x: 50, y: 50 });
    const child = testNode('text', 1, { x: 90, y: 100, groupId: group.id });
    const result = ungroupCanvasNodes(testDocument([child, group]), group.id);
    expect(result?.nodes).toHaveLength(1);
    expect(result?.nodes[0].position).toEqual({ x: 90, y: 100 });
    expect(result?.nodes[0].groupId).toBeNull();
  });
});

describe('Creative Studio clipboard', () => {
  test('copies a selected group with members and only internal connections', () => {
    const group = testNode('group', 3, { x: 50, y: 50 });
    const first = testNode('text', 1, { x: 90, y: 100, groupId: group.id });
    const second = testNode('image', 2, { x: 240, y: 100, groupId: group.id });
    const outside = testNode('video', 4, { x: 500, y: 100 });
    const internal = testEdge(10, first.id, second.id);
    const outbound = testEdge(11, second.id, outside.id);
    const clipboard = copyCanvasFragment(
      testDocument([first, second, group, outside], [internal, outbound]),
      [group.id]
    );

    expect(clipboard?.nodes.map((node) => node.id)).toEqual([first.id, second.id, group.id]);
    expect(clipboard?.connections).toEqual([internal]);
  });

  test('promotes a copied child when its group is not copied', () => {
    const group = testNode('group', 3);
    const child = testNode('text', 1, { x: 90, y: 100, groupId: group.id });
    const clipboard = copyCanvasFragment(testDocument([child, group]), [child.id]);
    expect(clipboard?.nodes).toHaveLength(1);
    expect(clipboard?.nodes[0].groupId).toBeNull();
  });

  test('pastes with fresh ids, remapped membership/endpoints, and a stable offset', () => {
    const group = testNode('group', 3, { x: 50, y: 50 });
    const first = testNode('text', 1, { x: 90, y: 100, groupId: group.id });
    const second = testNode('image', 2, { x: 240, y: 100, groupId: group.id });
    const clipboard = copyCanvasFragment(
      testDocument([first, second, group], [testEdge(10, first.id, second.id)]),
      [group.id]
    );
    if (!clipboard) throw new Error('expected clipboard');

    const pasted = materializeCanvasPaste(clipboard, {
      offset: { x: 32, y: -16 },
      idFactory: sequentialTestIdFactory(100),
    });
    expect(new Set(pasted.selectedIds).size).toBe(3);
    expect(pasted.selectedIds.some((id) => [first.id, second.id, group.id].includes(id))).toBe(false);

    const pastedGroup = pasted.nodes.find((node) => node.type === 'group');
    const pastedFirst = pasted.nodes.find((node) => node.type === 'text');
    const pastedSecond = pasted.nodes.find((node) => node.type === 'image');
    expect(pastedGroup?.position).toEqual({ x: 82, y: 34 });
    expect(pastedFirst?.position).toEqual({ x: 122, y: 84 });
    expect(pastedFirst?.groupId).toBe(pastedGroup?.id);
    expect(pastedSecond?.groupId).toBe(pastedGroup?.id);
    expect(pasted.connections).toEqual([
      {
        id: testUuid(103),
        sourceNodeId: pastedFirst?.id,
        targetNodeId: pastedSecond?.id,
        sourceHandle: null,
        targetHandle: null,
      },
    ]);
  });

  test('remaps durable image-prompt mentions when their source is pasted together', () => {
    const source = testNode('image', 20);
    const target = testNode('image', 21);
    target.data.composer = {
      prompt: '@人物图 出镜',
      mentions: [
        {
          id: 'mention-1',
          sourceNodeId: source.id,
          fallbackLabel: '人物图',
          start: 0,
          end: 4,
        },
      ],
      model: null,
      interfaceMode: 'images',
      quality: 'auto',
      width: 1024,
      height: 1024,
      aspectRatio: '1:1',
      count: 1,
    };
    const clipboard = copyCanvasFragment(
      testDocument([source, target], [testEdge(22, source.id, target.id)]),
      [source.id, target.id]
    );
    if (!clipboard) throw new Error('expected clipboard');

    const pasted = materializeCanvasPaste(clipboard, {
      idFactory: sequentialTestIdFactory(220),
    });
    const pastedSource = pasted.nodes[0];
    const pastedTarget = pasted.nodes[1];
    expect(pastedTarget?.type).toBe('image');
    expect(
      pastedTarget?.type === 'image'
        ? pastedTarget.data.composer?.mentions?.[0]?.sourceNodeId
        : null
    ).toBe(pastedSource?.id);
  });
});
