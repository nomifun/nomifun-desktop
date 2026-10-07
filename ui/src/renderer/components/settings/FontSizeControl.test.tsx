/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import * as themeContext from '@renderer/hooks/context/ThemeContext';
import FontSizeControl from './FontSizeControl';
import settings from '@renderer/services/i18n/locales/en-US/settings.json';

const locale = createInstance();
await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } }, interpolation: { escapeValue: false } });
const setFontScale = mock(async (_scale: number) => {});
const value = {
  theme: 'light' as const,
  colorScheme: 'default' as const,
  fontScale: 1,
  setTheme: async () => {},
  setColorScheme: async () => {},
  setFontScale,
};
let themeSpy: ReturnType<typeof spyOn<typeof themeContext, 'useThemeContext'>>;

beforeEach(() => {
  value.fontScale = 1;
  setFontScale.mockClear();
  themeSpy = spyOn(themeContext, 'useThemeContext').mockReturnValue(value);
});
afterEach(() => {
  cleanup();
  themeSpy.mockRestore();
});

const control = () => <I18nextProvider i18n={locale}><FontSizeControl /></I18nextProvider>;

test('typing defers zoom until Enter or blur, then snaps to the supported step and bounds', () => {
  const view = render(control());
  const input = view.getByRole('textbox', { name: 'Scale' }) as HTMLInputElement;
  act(() => input.focus());
  fireEvent.change(input, { target: { value: '129' } });
  expect(setFontScale).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(setFontScale).toHaveBeenLastCalledWith(1.3);
  expect(input.value).toBe('130');

  fireEvent.change(input, { target: { value: '1' } });
  fireEvent.blur(input);
  expect(setFontScale).toHaveBeenLastCalledWith(0.8);
  expect(input.value).toBe('80');

  fireEvent.change(input, { target: { value: '113' } });
  fireEvent.blur(input);
  expect(setFontScale).toHaveBeenLastCalledWith(1.15);
});

test('invalid values and cancelled edits preserve the applied zoom', () => {
  const view = render(control());
  const input = view.getByRole('textbox', { name: 'Scale' }) as HTMLInputElement;
  for (const text of ['', 'abc', '1.2']) {
    fireEvent.change(input, { target: { value: text } });
    fireEvent.blur(input);
    expect(input.value).toBe('100');
  }
  fireEvent.change(input, { target: { value: '125' } });
  fireEvent.keyDown(input, { key: 'Escape' });
  fireEvent.blur(input);
  expect(input.value).toBe('100');
  expect(setFontScale).not.toHaveBeenCalled();
});

test('the input and disabled controls follow persisted zoom updates and rollback', () => {
  const view = render(control());
  const input = view.getByRole('textbox', { name: 'Scale' }) as HTMLInputElement;
  value.fontScale = 1.3;
  view.rerender(control());
  expect(input.value).toBe('130');
  expect((view.getByRole('button', { name: 'Zoom in' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('button', { name: 'Reset zoom' }));
  expect(setFontScale).toHaveBeenLastCalledWith(1);

  value.fontScale = 0.8;
  view.rerender(control());
  expect(input.value).toBe('80');
  expect((view.getByRole('button', { name: 'Zoom out' }) as HTMLButtonElement).disabled).toBe(true);

  value.fontScale = 1;
  view.rerender(control());
  expect(input.value).toBe('100');
  expect((view.getByRole('button', { name: 'Reset zoom' }) as HTMLButtonElement).disabled).toBe(true);
});
