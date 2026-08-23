/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import React, { useState } from 'react';

import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CreativeCanvasChrome from './CreativeCanvasChrome';
import type { CreativeCanvasChromeProps } from './types';

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
  isMiniMapOpen: false,
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
