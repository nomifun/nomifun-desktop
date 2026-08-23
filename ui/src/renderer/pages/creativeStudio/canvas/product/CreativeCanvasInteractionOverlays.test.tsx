/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import CreativeCanvasInteractionOverlays, {
  type CreativeCanvasInteractionOverlaysProps,
} from './CreativeCanvasInteractionOverlays';

const noop = () => undefined;

const renderOverlay = (
  overrides: Partial<CreativeCanvasInteractionOverlaysProps>
): string =>
  renderToStaticMarkup(
    <CreativeCanvasInteractionOverlays
      viewportSize={{ width: 900, height: 640 }}
      contextMenu={null}
      createNodeMenu={null}
      onContextAction={noop}
      onOpenCreateNodeMenu={noop}
      onPasteFromSystemClipboard={noop}
      onSelectNode={noop}
      onDismiss={noop}
      {...overrides}
    />
  );

describe('CreativeCanvasInteractionOverlays', () => {
  test('renders source canvas actions at a clamped screen-space position', () => {
    const html = renderOverlay({
      contextMenu: {
        target: { kind: 'canvas' },
        clientPosition: { x: 895, y: 635 },
      },
    });

    expect(html.includes('画布上下文菜单')).toBe(true);
    expect(html.includes('添加节点')).toBe(true);
    expect(html.includes('从系统剪贴板粘贴')).toBe(true);
    expect(html.includes('left:704px')).toBe(true);
    expect(html.includes('top:524px')).toBe(true);
  });

  test('renders lock-aware node actions and edge deletion without canvas actions', () => {
    const node = renderOverlay({
      contextMenu: {
        target: { kind: 'node', nodeId: 'node-1' },
        clientPosition: { x: 20, y: 30 },
        nodeLocked: true,
      },
    });
    expect(node.includes('打开')).toBe(true);
    expect(node.includes('创建副本')).toBe(true);
    expect(node.includes('解锁节点')).toBe(true);
    expect(node.includes('删除节点')).toBe(true);
    expect(node.includes('从系统剪贴板粘贴')).toBe(false);

    const edge = renderOverlay({
      contextMenu: {
        target: { kind: 'edge', edgeId: 'edge-1' },
        clientPosition: { x: 20, y: 30 },
      },
    });
    expect(edge.includes('删除连接')).toBe(true);
    expect(edge.includes('创建副本')).toBe(false);
  });

  test('reuses the canonical eight-kind node menu and renders nothing when closed', () => {
    const menu = renderOverlay({
      createNodeMenu: { clientPosition: { x: 100, y: 120 } },
    });
    expect((menu.match(/data-node-kind=/g) ?? []).length).toBe(8);
    expect(renderOverlay({})).toBe('');
  });
});
