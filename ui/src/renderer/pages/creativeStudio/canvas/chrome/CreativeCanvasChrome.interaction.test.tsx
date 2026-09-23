/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import React, { useState } from 'react';

import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CreativeCanvasChrome from './CreativeCanvasChrome';
import type {
  CreativeCanvasBottomView,
  CreativeCanvasChromeProps,
  CreativeCanvasChromeTool,
} from './types';

const noop = () => undefined;

const baseProps = (
  overrides: Partial<CreativeCanvasChromeProps> = {}
): CreativeCanvasChromeProps => ({
  canvasTitle: '交互测试画布',
  saveStatus: 'saved',
  tool: 'select',
  background: 'lines',
  canUndo: false,
  canRedo: false,
  leftOpen: true,
  leftView: 'canvas',
  resourceView: null,
  rightView: null,
  bottomView: null,
  backgroundMenuOpen: false,
  slots: {
    canvas: <div>canvas</div>,
    left: {
      canvas: <div>outline</div>,
      assets: <div>assets</div>,
    },
    bottom: {
      history: <div>HISTORY CONTENT</div>,
    },
  },
  onBackToCanvases: noop,
  onToolChange: noop,
  onAddNode: noop,
  onBackgroundChange: noop,
  onBackgroundMenuOpenChange: noop,
  onUndo: noop,
  onRedo: noop,
  onLeftPanelOpenChange: noop,
  onLeftViewChange: noop,
  onResourceViewChange: noop,
  onRightViewChange: noop,
  onBottomViewChange: noop,
  ...overrides,
});

afterEach(() => {
  cleanup();
});

test('switching canvases discards the previous title edit even when names match', () => {
  const props = baseProps({ canvasId: 'first', onRenameCanvas: async () => {} });
  const view = render(withCanvasTestI18n(<CreativeCanvasChrome {...props} />));
  fireEvent.doubleClick(view.getByRole('heading', { name: props.canvasTitle }));
  fireEvent.change(view.getByRole('textbox'), { target: { value: 'Unsubmitted title' } });
  view.rerender(withCanvasTestI18n(<CreativeCanvasChrome {...props} canvasId='second' />));
  expect(view.queryByRole('textbox')).toBeNull();
  expect(view.getByRole('heading', { name: props.canvasTitle })).toBeTruthy();
});

describe('CreativeCanvasChrome floating resource rail interaction', () => {
  test('only the canvas control owns the rail active state', () => {
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasChrome
          {...baseProps({
            leftOpen: false,
            leftView: 'assets',
          })}
        />
      )
    );

    const rail = getByRole('toolbar', {
      name: 'creativeStudio.canvas.chrome.resources',
    });
    expect(within(rail).getByRole('button', {
      name: 'creativeStudio.canvas.panels.left.canvas',
    }).getAttribute('aria-pressed')).toBe('false');
    for (const label of ['assets', 'prompts', 'templates']) {
      const button = within(rail).getByRole('button', {
        name: `creativeStudio.canvas.panels.left.${label}`,
      });
      expect(button.getAttribute('aria-expanded')).toBe('false');
      expect(button.hasAttribute('data-active')).toBe(false);
    }
  });

  test('collapses the canvas panel from its control and the dedicated fold control', () => {
    const openChanges: boolean[] = [];
    const { getByRole, getByLabelText } = render(
      withCanvasTestI18n(
        <CreativeCanvasChrome
          {...baseProps({
            onLeftPanelOpenChange: (open) => {
              openChanges.push(open);
            },
          })}
        />
      )
    );

    fireEvent.click(
      getByRole('button', {
        name: 'creativeStudio.canvas.panels.left.canvas',
      })
    );
    expect(openChanges).toEqual([false]);

    fireEvent.click(
      getByLabelText('creativeStudio.canvas.chrome.collapseResources')
    );
    expect(openChanges).toEqual([false, false]);
  });

  test('keeps node creation buttons from reopening the canvas bubble', () => {
    const created: string[] = [];
    const NodeHarness: React.FC = () => {
      const [leftOpen, setLeftOpen] = useState(true);
      return (
        <CreativeCanvasChrome
          {...baseProps({
            leftOpen,
            onLeftPanelOpenChange: setLeftOpen,
            onAddNode: (kind) => created.push(kind),
          })}
        />
      );
    };

    const { getByRole, container } = render(withCanvasTestI18n(<NodeHarness />));
    fireEvent.click(
      getByRole('button', { name: 'creativeStudio.canvas.nodeKinds.text' })
    );

    expect(created).toEqual(['text']);
    expect(container.querySelector('[data-left-open="false"]')).not.toBeNull();
  });

  test('opens assets, prompts, and templates in the shared resource dialog', () => {
    const resourceChanges: Array<string | null> = [];
    const PanelHarness: React.FC = () => {
      const [leftOpen, setLeftOpen] = useState(false);
      const [resourceView, setResourceView] = useState<'assets' | 'prompts' | 'templates' | null>(null);

      return (
        <CreativeCanvasChrome
          {...baseProps({
            leftOpen,
            leftView: 'canvas',
            resourceView,
            onLeftPanelOpenChange: setLeftOpen,
            onResourceViewChange: (view) => {
              resourceChanges.push(view);
              setResourceView(view);
            },
          })}
        />
      );
    };

    const { getByRole } = render(withCanvasTestI18n(<PanelHarness />));

    fireEvent.click(
      getByRole('button', {
        name: 'creativeStudio.canvas.panels.left.assets',
      })
    );

    expect(resourceChanges).toEqual(['assets']);
    expect(
      document.querySelector('[data-canvas-resource-dialog="assets"]')?.textContent
    ).toContain('assets');
    expect(getByRole('button', {
      name: 'creativeStudio.canvas.panels.left.assets',
    }).hasAttribute('data-active')).toBe(false);
  });
});

describe('CreativeCanvasChrome top action interactions', () => {
  test('uses one hand toggle and one top-right entry for the shared bottom panel', () => {
    const ToolbarHarness: React.FC = () => {
      const [tool, setTool] = useState<CreativeCanvasChromeTool>('select');
      const [bottomView, setBottomView] =
        useState<CreativeCanvasBottomView | null>(null);

      return (
        <CreativeCanvasChrome
          {...baseProps({
            tool,
            bottomView,
            onToolChange: setTool,
            onBottomViewChange: setBottomView,
          })}
        />
      );
    };

    const { container, getByRole } = render(
      withCanvasTestI18n(<ToolbarHarness />)
    );
    expect(
      container.querySelector('[aria-label="creativeStudio.canvas.actions.selectTool"]')
    ).toBeNull();
    expect(
      container.querySelector('[aria-label="creativeStudio.canvas.actions.fitView"]')
    ).toBeNull();
    expect(
      container.querySelector('[aria-label="creativeStudio.canvas.actions.openMiniMap"]')
    ).toBeNull();
    expect(
      container.querySelector('[aria-label="creativeStudio.canvas.panels.bottom.timeline"]')
    ).toBeNull();

    const panButton = getByRole('button', {
      name: 'creativeStudio.canvas.actions.panTool',
    });
    expect(panButton.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(panButton);
    expect(panButton.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(panButton);
    expect(panButton.getAttribute('aria-pressed')).toBe('false');

    const historyButton = getByRole('button', {
      name: 'creativeStudio.canvas.panels.bottom.history',
    });
    expect(historyButton.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(historyButton);

    const historyPanel = container.querySelector<HTMLElement>(
      'section[aria-label="creativeStudio.canvas.panels.bottom.history"]'
    );
    expect(historyPanel).not.toBeNull();
    expect(historyButton.getAttribute('aria-pressed')).toBe('true');
    expect(historyPanel?.textContent?.includes('HISTORY CONTENT')).toBe(true);

    fireEvent.click(historyButton);
    expect(
      container.querySelector(
        'section[aria-label="creativeStudio.canvas.panels.bottom.history"]'
      )
    ).toBeNull();
    expect(historyButton.getAttribute('aria-pressed')).toBe('false');
  });

  test('keeps direct node creation in source order on the side rail', () => {
    const created: string[] = [];
    const { getByRole } = render(withCanvasTestI18n(
      <CreativeCanvasChrome {...baseProps({ onAddNode: (kind) => created.push(kind) })} />
    ));
    const rail = getByRole('toolbar', {
      name: 'creativeStudio.canvas.chrome.resources',
    });
    for (const kind of ['text', 'image', 'video', 'audio', 'timeline']) {
      fireEvent.click(within(rail).getByRole('button', {
        name: `creativeStudio.canvas.nodeKinds.${kind}`,
      }));
    }
    expect(created).toEqual(['text', 'image', 'video', 'audio', 'timeline']);
  });
});
describe('CreativeCanvasChrome right panel resize interaction', () => {
  test('adjusts the persisted width with keyboard controls', () => {
    const widthChanges: number[] = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasChrome
          {...baseProps({
            rightView: 'assistant',
            rightPanelWidth: 390,
            onRightPanelWidthChange: (width) => widthChanges.push(width),
          })}
        />
      )
    );

    const separator = getByRole('separator', {
      name: 'creativeStudio.canvas.chrome.resizeRightPanel',
    });
    fireEvent.keyDown(separator, { key: 'ArrowLeft' });
    expect(widthChanges).toEqual([406]);

    fireEvent.keyDown(separator, { key: 'End' });
    expect(widthChanges).toEqual([406, 320]);
  });

  test('updates the draft during pointer drag and commits the final width', () => {
    const widthChanges: number[] = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasChrome
          {...baseProps({
            rightView: 'assistant',
            rightPanelWidth: 390,
            onRightPanelWidthChange: (width) => widthChanges.push(width),
          })}
        />
      )
    );

    const separator = getByRole('separator', {
      name: 'creativeStudio.canvas.chrome.resizeRightPanel',
    });
    fireEvent.pointerDown(separator, {
      button: 0,
      clientX: 100,
      pointerId: 1,
      pointerType: 'mouse',
    });
    fireEvent.pointerMove(window, {
      buttons: 1,
      clientX: 0,
      pointerId: 1,
    });
    fireEvent.pointerUp(window, {
      clientX: 0,
      pointerId: 1,
    });

    expect(widthChanges.at(-1)).toBe(490);
  });
});
