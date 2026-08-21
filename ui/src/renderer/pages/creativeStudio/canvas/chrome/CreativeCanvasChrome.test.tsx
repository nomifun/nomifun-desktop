/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import CreativeCanvasChrome, {
  CreativeCanvasBackgroundMenu,
  CreativeCanvasNodeMenu,
} from './CreativeCanvasChrome';
import {
  CREATIVE_CANVAS_CHROME_BACKGROUNDS,
  CREATIVE_CANVAS_CHROME_NODE_KINDS,
  CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS,
  toggleCreativeCanvasPanel,
  type CreativeCanvasChromeProps,
} from './types';

const noop = () => undefined;

const baseProps = (
  overrides: Partial<CreativeCanvasChromeProps> = {}
): CreativeCanvasChromeProps => ({
  projectTitle: '品牌概念画布',
  saveStatus: 'saved',
  tool: 'select',
  background: 'dots',
  canUndo: false,
  canRedo: true,
  isMiniMapOpen: false,
  leftView: 'canvas',
  rightView: 'assistant',
  bottomView: 'history',
  backgroundMenuOpen: false,
  compact: false,
  slots: {
    canvas: <div data-test-slot='canvas'>CANVAS SLOT</div>,
    topActions: <span>TOP ACTION</span>,
    toolbarTrailing: <span>TRAILING TOOL</span>,
    left: { canvas: <div>CANVAS PANEL</div> },
    right: { assistant: <div>ASSISTANT PANEL</div> },
    bottom: { history: <div>HISTORY PANEL</div> },
  },
  onBackToProjects: noop,
  onToolChange: noop,
  onAddNode: noop,
  onBackgroundChange: noop,
  onBackgroundMenuOpenChange: noop,
  onUndo: noop,
  onRedo: noop,
  onFitView: noop,
  onToggleMiniMap: noop,
  onLeftViewChange: noop,
  onRightViewChange: noop,
  onBottomViewChange: noop,
  ...overrides,
});

const renderChrome = (overrides: Partial<CreativeCanvasChromeProps> = {}) =>
  renderToStaticMarkup(<CreativeCanvasChrome {...baseProps(overrides)} />);

describe('CreativeCanvasChrome source-shaped layout', () => {
  test('renders project toolbar, real canvas column, panels, and controlled tool actions', () => {
    const html = renderChrome();

    expect(html.includes('data-creative-canvas-chrome="true"')).toBe(true);
    expect(html.includes('data-canvas-chrome-stage="true"')).toBe(true);
    expect(html.includes('品牌概念画布')).toBe(true);
    expect(html.includes('已保存')).toBe(true);
    expect(html.includes('返回项目')).toBe(true);
    expect(html.includes('CANVAS SLOT')).toBe(true);
    expect(html.includes('CANVAS PANEL')).toBe(true);
    expect(html.includes('ASSISTANT PANEL')).toBe(true);
    expect(html.includes('HISTORY PANEL')).toBe(true);
    expect(html.includes('aria-label="选择工具"')).toBe(true);
    expect(html.includes('aria-label="平移工具"')).toBe(true);
    expect(html.includes('aria-label="撤销"')).toBe(true);
    expect(html.includes('aria-label="重做"')).toBe(true);
    expect(html.includes('aria-label="适应内容"')).toBe(true);
    expect(html.includes('aria-label="打开小地图"')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('data-right-panel-header')).toBe(false);
    expect(html.includes('添加节点')).toBe(false);
    expect(CREATIVE_CANVAS_CHROME_TOOLBAR_NODE_KINDS).toEqual([
      'text',
      'image',
      'video',
      'audio',
      'panorama',
      'director',
      'config',
    ]);
    for (const label of [
      '文本',
      '图片',
      '视频',
      '音频',
      '全景图',
      '导演台',
      '生成配置',
    ]) {
      expect(html.includes(`aria-label="${label}"`)).toBe(true);
    }
    expect(html.includes('aria-label="分组"')).toBe(false);
  });

  test('keeps a header for properties while the Agent owns its source header', () => {
    const properties = renderChrome({
      rightView: 'properties',
      slots: {
        ...baseProps().slots,
        right: { properties: <div>PROPERTIES PANEL</div> },
      },
    });

    expect(properties.includes('data-right-panel-header="properties"')).toBe(true);
    expect(properties.includes('PROPERTIES PANEL')).toBe(true);
  });

  test('renders four left views, two right views, and two bottom views', () => {
    const html = renderChrome({ compact: true });

    for (const label of [
      '画布',
      '资产',
      '提示词',
      '工作流',
      '创作助手',
      '属性',
      '历史',
      '时间线',
    ]) {
      expect(html.includes(label)).toBe(true);
    }
    expect(html.includes('data-compact="true"')).toBe(true);
  });

  test('exposes save conflict as an alert and omits closed optional panels', () => {
    const html = renderChrome({
      saveStatus: 'conflict',
      saveMessage: '远端版本已更新',
      rightView: null,
      bottomView: null,
    });

    expect(html.includes('data-save-status="conflict"')).toBe(true);
    expect(html.includes('role="alert"')).toBe(true);
    expect(html.includes('远端版本已更新')).toBe(true);
    expect(html.includes('data-right-view="closed"')).toBe(true);
    expect(html.includes('data-bottom-view="closed"')).toBe(true);
    expect(html.includes('data-right-panel-body')).toBe(false);
    expect(html.includes('data-bottom-panel-body')).toBe(false);
  });
});

describe('CreativeCanvasChrome controlled menus', () => {
  test('offers exactly the eight canonical node kinds', () => {
    const html = renderToStaticMarkup(<CreativeCanvasNodeMenu onSelect={noop} />);

    expect(CREATIVE_CANVAS_CHROME_NODE_KINDS).toHaveLength(8);
    expect((html.match(/data-node-kind=/g) ?? []).length).toBe(8);
    for (const kind of CREATIVE_CANVAS_CHROME_NODE_KINDS) {
      expect(html.includes(`data-node-kind="${kind}"`)).toBe(true);
    }
  });

  test('offers only dots, lines, and blank background modes', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasBackgroundMenu value='lines' onChange={noop} />
    );

    expect(CREATIVE_CANVAS_CHROME_BACKGROUNDS).toEqual(['dots', 'lines', 'blank']);
    expect((html.match(/data-background=/g) ?? []).length).toBe(3);
    expect(html.includes('data-background="dots"')).toBe(true);
    expect(html.includes('data-background="lines"')).toBe(true);
    expect(html.includes('data-background="blank"')).toBe(true);
    expect(html.includes('aria-checked="true"')).toBe(true);
  });

  test('toggles optional panels without keeping internal product state', () => {
    expect(toggleCreativeCanvasPanel(null, 'assistant')).toBe('assistant');
    expect(toggleCreativeCanvasPanel('assistant', 'assistant')).toBe(null);
    expect(toggleCreativeCanvasPanel('assistant', 'properties')).toBe('properties');
  });
});
