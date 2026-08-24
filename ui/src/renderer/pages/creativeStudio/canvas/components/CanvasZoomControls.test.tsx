/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import { withCanvasTestI18n } from './canvasI18nTestUtils';
import CanvasZoomControls from './CanvasZoomControls';

afterEach(() => {
  cleanup();
});

describe('CanvasZoomControls', () => {
  test('opens the compact percentage menu with background modes and a numeric stepper', () => {
    const { getByRole } = render(
      withCanvasTestI18n(
        <CanvasZoomControls
          zoom={0.82}
          background='dots'
          onZoomChange={() => undefined}
          onBackgroundChange={() => undefined}
          onFitView={() => undefined}
          onToggleMiniMap={() => undefined}
        />
      )
    );

    const trigger = getByRole('button', {
      name: 'creativeStudio.canvas.zoom.slider, 82%',
    });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(trigger);

    const menu = getByRole('menu');
    expect(within(menu).getByRole('menuitemradio', {
      name: 'creativeStudio.canvas.backgrounds.dots',
    }).getAttribute('aria-checked')).toBe('true');
    expect(within(menu).getByRole('menuitemradio', {
      name: 'creativeStudio.canvas.backgrounds.lines',
    }).getAttribute('aria-checked')).toBe('false');
    expect(within(menu).getByRole('textbox').getAttribute('value')).toBe('82');
    expect(within(menu).getByRole('button', {
      name: 'creativeStudio.canvas.zoom.zoomOut',
    })).toBeDefined();
    expect(within(menu).getByRole('button', {
      name: 'creativeStudio.canvas.zoom.zoomIn',
    })).toBeDefined();
  });

  test('commits a typed percentage and moves background selection into the menu', () => {
    const zoomChanges: number[] = [];
    const backgroundChanges: string[] = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CanvasZoomControls
          zoom={0.82}
          background='lines'
          onZoomChange={(zoom) => zoomChanges.push(zoom)}
          onBackgroundChange={(background) => backgroundChanges.push(background)}
        />
      )
    );

    fireEvent.click(getByRole('button', {
      name: 'creativeStudio.canvas.zoom.slider, 82%',
    }));
    const menu = getByRole('menu');
    const input = within(menu).getByRole('textbox');
    (input as HTMLInputElement).value = '125';
    fireEvent.blur(input);
    expect(zoomChanges.at(-1)).toBe(1.25);

    fireEvent.click(within(menu).getByRole('menuitemradio', {
      name: 'creativeStudio.canvas.backgrounds.blank',
    }));
    expect(backgroundChanges).toEqual(['blank']);
  });

  test('keeps the minimap variant compact and dismisses the menu with Escape', () => {
    const { container, getByRole, queryByRole } = render(
      withCanvasTestI18n(
        <CanvasZoomControls
          zoom={0.82}
          showInlineStepper
          onZoomChange={() => undefined}
          onBackgroundChange={() => undefined}
        />
      )
    );

    expect(
      container.querySelector('[data-canvas-zoom-controls]')?.getAttribute('data-embedded')
    ).toBe('true');

    fireEvent.click(getByRole('button', {
      name: 'creativeStudio.canvas.zoom.slider, 82%',
    }));
    expect(getByRole('menu')).toBeDefined();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(queryByRole('menu')).toBeNull();
  });
});
