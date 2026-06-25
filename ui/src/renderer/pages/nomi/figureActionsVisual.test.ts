/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Custom figure card action polish', () => {
  test('uses the shared glass action surface in both figure library and character picker', () => {
    const library = readSource(new URL('./FigureLibraryPage.tsx', import.meta.url));
    const picker = readSource(new URL('./CharacterPicker.tsx', import.meta.url));
    const actions = readSource(new URL('./FigureCardActions.tsx', import.meta.url));

    for (const source of [library, picker]) {
      expect(source.includes('FigureActionSurface')).toBe(true);
      expect(source.includes('FigureActionButton')).toBe(true);
      expect(source.includes('bg-[var(--color-bg-5)]')).toBe(false);
    }

    expect(actions.includes('figure-card-action-surface')).toBe(true);
    expect(actions.includes('figure-card-action-button')).toBe(true);
    expect(actions.includes('backdrop-blur')).toBe(true);
  });

  test('opens a full edit modal instead of the old inline rename-only flow', () => {
    const library = readSource(new URL('./FigureLibraryPage.tsx', import.meta.url));
    const figuresHook = readSource(new URL('./useFigures.ts', import.meta.url));
    const ipcBridge = readSource(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url));

    expect(library.includes('FigureEditModal')).toBe(true);
    expect(library.includes('onUpdate={update}')).toBe(true);
    expect(library.includes('FigureInlineConfirmButton')).toBe(false);
    expect(library.includes('w-22px h-22px rd-full')).toBe(false);
    expect(figuresHook.includes('update: (id: string, patch: FigureUpdatePatch) => Promise<IFigureMeta>')).toBe(true);
    expect(ipcBridge.includes('updateFigure')).toBe(true);
  });

  test('figure edit modal reuses the creation framing controls for crop and size', () => {
    const modalUrl = new URL('./FigureEditModal.tsx', import.meta.url);
    expect(existsSync(modalUrl)).toBe(true);
    const modal = readSource(modalUrl);

    expect(modal.includes('FrameStep')).toBe(true);
    expect(modal.includes('fig.head_box')).toBe(true);
    expect(modal.includes('fig.size_tier')).toBe(true);
    expect(modal.includes('onSave')).toBe(true);
  });

  test('creation and edit modal footer actions are compact and right aligned', () => {
    const wizard = readSource(new URL('./CustomFigureWizard/index.tsx', import.meta.url));
    const editModal = readSource(new URL('./FigureEditModal.tsx', import.meta.url));

    for (const source of [wizard, editModal]) {
      expect(source.includes('figure-modal-footer flex items-center justify-end gap-8px pt-10px')).toBe(true);
      expect(source.includes('border-t border-solid border-[var(--color-border-2)]')).toBe(false);
      expect(source.includes('gap-12px pt-14px mt-2px')).toBe(false);
    }
  });
});
