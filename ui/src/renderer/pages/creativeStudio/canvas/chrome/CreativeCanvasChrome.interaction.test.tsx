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
      timeline: <div>TIMELINE CONTENT</div>,
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
  onRightViewChange: noop,
  onBottomViewChange: noop,
  ...overrides,
});

afterEach(() => {
  cleanup();
});

describe('CreativeCanvasChrome floating resource rail interaction', () => {
  test('does not mark a collapsed resource rail tab as active', () => {
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

    for (const tab of getByRole('tablist', {
      name: 'creativeStudio.canvas.chrome.resources',
    }).querySelectorAll('[role="tab"]')) {
      expect(tab.getAttribute('aria-selected')).toBe('false');
      expect(tab.hasAttribute('data-active')).toBe(false);
    }
  });

  test('collapses from the active tab and the dedicated fold control', () => {
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
      getByRole('tab', {
        name: 'creativeStudio.canvas.panels.left.canvas',
      })
    );
    expect(openChanges).toEqual([false]);

    fireEvent.click(
      getByLabelText('creativeStudio.canvas.chrome.collapseResources')
    );
    expect(openChanges).toEqual([false, false]);
  });

  test('activating another bubble switches its view and reopens the content panel', () => {
    const viewChanges: string[] = [];
    const PanelHarness: React.FC = () => {
      const [leftOpen, setLeftOpen] = useState(false);
      const [leftView, setLeftView] = useState<'canvas' | 'assets'>('canvas');

      return (
        <CreativeCanvasChrome
          {...baseProps({
            leftOpen,
            leftView,
            onLeftPanelOpenChange: setLeftOpen,
            onLeftViewChange: (view) => {
              viewChanges.push(view);
              if (view === 'canvas' || view === 'assets') setLeftView(view);
              setLeftOpen(true);
            },
          })}
        />
      );
    };

    const { getByRole } = render(withCanvasTestI18n(<PanelHarness />));

    fireEvent.click(
      getByRole('tab', {
        name: 'creativeStudio.canvas.panels.left.assets',
      })
    );

    expect(viewChanges).toEqual(['assets']);
    expect(getByRole('tabpanel').hasAttribute('hidden')).toBe(false);
  });
});

describe('CreativeCanvasChrome toolbar interactions', () => {
  test('uses one hand toggle and one entry for the shared bottom panel', () => {
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
    const toolbar = getByRole('toolbar', {
      name: 'creativeStudio.canvas.chrome.toolbar',
    });

    expect(
      within(toolbar).queryByRole('button', {
        name: 'creativeStudio.canvas.actions.selectTool',
      })
    ).toBeNull();
    expect(
      within(toolbar).queryByRole('button', {
        name: 'creativeStudio.canvas.actions.fitView',
      })
    ).toBeNull();
    expect(
      within(toolbar).queryByRole('button', {
        name: 'creativeStudio.canvas.actions.openMiniMap',
      })
    ).toBeNull();
    expect(
      within(toolbar).queryByRole('button', {
        name: 'creativeStudio.canvas.panels.bottom.timeline',
      })
    ).toBeNull();

    const panButton = within(toolbar).getByRole('button', {
      name: 'creativeStudio.canvas.actions.panTool',
    });
    expect(panButton.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(panButton);
    expect(panButton.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(panButton);
    expect(panButton.getAttribute('aria-pressed')).toBe('false');

    const historyButton = within(toolbar).getByRole('button', {
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

    fireEvent.click(
      within(historyPanel!).getByRole('tab', {
        name: 'creativeStudio.canvas.panels.bottom.timeline',
      })
    );
    const timelinePanel = container.querySelector<HTMLElement>(
      'section[aria-label="creativeStudio.canvas.panels.bottom.timeline"]'
    );
    expect(timelinePanel).not.toBeNull();
    expect(timelinePanel?.textContent?.includes('TIMELINE CONTENT')).toBe(true);
    expect(historyButton.getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(historyButton);
    expect(
      container.querySelector(
        'section[aria-label="creativeStudio.canvas.panels.bottom.history"], section[aria-label="creativeStudio.canvas.panels.bottom.timeline"]'
      )
    ).toBeNull();
    expect(historyButton.getAttribute('aria-pressed')).toBe('false');
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
