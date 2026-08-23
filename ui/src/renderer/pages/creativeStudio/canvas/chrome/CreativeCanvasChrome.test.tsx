/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
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
  canvasTitle: '品牌概念画布',
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
  onBackToCanvases: noop,
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
  renderToStaticMarkup(
    withCanvasTestI18n(<CreativeCanvasChrome {...baseProps(overrides)} />)
  );

describe('CreativeCanvasChrome source-shaped layout', () => {
  test('renders the canvas toolbar, real canvas column, panels, and controlled tool actions', () => {
    const html = renderChrome();

    expect(html.includes('data-creative-canvas-chrome="true"')).toBe(true);
    expect(html.includes('data-canvas-chrome-stage="true"')).toBe(true);
    expect(html.includes('品牌概念画布')).toBe(true);
    expect(
      html.includes('creativeStudio.canvas.save.status.saved')
    ).toBe(true);
    expect(
      html.includes('creativeStudio.canvas.chrome.backToLibrary')
    ).toBe(true);
    expect(html.includes('CANVAS SLOT')).toBe(true);
    expect(html.includes('CANVAS PANEL')).toBe(true);
    expect(html.includes('ASSISTANT PANEL')).toBe(true);
    expect(html.includes('HISTORY PANEL')).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.selectTool"')
    ).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.panTool"')
    ).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.undo"')
    ).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.redo"')
    ).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.fitView"')
    ).toBe(true);
    expect(
      html.includes('aria-label="creativeStudio.canvas.actions.openMiniMap"')
    ).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('data-right-panel-header')).toBe(false);
    expect(
      html.includes('creativeStudio.canvas.chrome.addNode')
    ).toBe(false);
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
      'creativeStudio.canvas.nodeKinds.text',
      'creativeStudio.canvas.nodeKinds.image',
      'creativeStudio.canvas.nodeKinds.video',
      'creativeStudio.canvas.nodeKinds.audio',
      'creativeStudio.canvas.nodeKinds.panorama',
      'creativeStudio.canvas.nodeKinds.director',
      'creativeStudio.canvas.nodeKinds.config',
    ]) {
      expect(html.includes(`aria-label="${label}"`)).toBe(true);
    }
    expect(
      html.includes('aria-label="creativeStudio.canvas.nodeKinds.group"')
    ).toBe(false);
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
      'creativeStudio.canvas.panels.left.canvas',
      'creativeStudio.canvas.panels.left.assets',
      'creativeStudio.canvas.panels.left.prompts',
      'creativeStudio.canvas.panels.left.workflows',
      'creativeStudio.canvas.panels.right.assistant',
      'creativeStudio.canvas.panels.right.properties',
      'creativeStudio.canvas.panels.bottom.history',
      'creativeStudio.canvas.panels.bottom.timeline',
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
    const html = renderToStaticMarkup(
      withCanvasTestI18n(<CreativeCanvasNodeMenu onSelect={noop} />)
    );

    expect(CREATIVE_CANVAS_CHROME_NODE_KINDS).toHaveLength(8);
    expect((html.match(/data-node-kind=/g) ?? []).length).toBe(8);
    for (const kind of CREATIVE_CANVAS_CHROME_NODE_KINDS) {
      expect(html.includes(`data-node-kind="${kind}"`)).toBe(true);
    }
  });

  test('offers only dots, lines, and blank background modes', () => {
    const html = renderToStaticMarkup(
      withCanvasTestI18n(
        <CreativeCanvasBackgroundMenu value='lines' onChange={noop} />
      )
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
