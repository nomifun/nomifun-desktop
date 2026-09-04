/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import i18next from 'i18next';
import React from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { CreativeAsset } from '../types';
import { createCreativeAssetLibraryLabels } from './types';

// Arco snapshots DOM availability when its CommonJS entry is evaluated.
// Load the menu after setup-dom has installed the browser globals.
const { default: CreativeAssetActionsMenu } = await import('./CreativeAssetActionsMenu');

const testI18n = i18next.createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
});
const labels = createCreativeAssetLibraryLabels(testI18n.t.bind(testI18n));
const asset: CreativeAsset = {
  id: 'asset-menu-video', kind: 'video', title: '竖屏素材', collection: null, tags: [],
  mimeType: 'video/mp4', width: 720, height: 1280, bytes: 2048, inLibrary: true,
  textContent: null, origin: null, originalUrl: '/clip.mp4', thumbnailUrl: '/cover.jpg',
  createdAt: 1_777_000_000_000, updatedAt: 1_777_000_000_000,
};

type MenuProps = React.ComponentProps<typeof CreativeAssetActionsMenu>;
const menu = (props: Partial<MenuProps> = {}) => (
  <I18nextProvider i18n={testI18n}>
    <CreativeAssetActionsMenu
      asset={asset}
      labels={labels}
      disabled={false}
      onOpen={() => undefined}
      onEdit={() => undefined}
      onDownload={() => undefined}
      onRemove={() => undefined}
      {...props}
    />
  </I18nextProvider>
);

const openMenu = async (trigger: HTMLButtonElement) => {
  fireEvent.click(trigger);
  await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('true'));
  return within(document.body).findByRole('menu');
};

const expectClosed = async (trigger: HTMLButtonElement) => {
  await waitFor(() => {
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(within(document.body).queryByRole('menu') === null).toBe(true);
  });
};

afterEach(() => {
  cleanup();
  document.getElementById('creative-studio-portal-root')?.remove();
});

describe('CreativeAssetActionsMenu', () => {
  test('dispatches each action once with the original asset, then closes the menu', async () => {
    const calls: Array<{ action: string; asset: CreativeAsset }> = [];
    const { getByRole } = render(menu({
      onOpen: (value) => calls.push({ action: 'open', asset: value }),
      onEdit: (value) => calls.push({ action: 'edit', asset: value }),
      onDownload: (value) => calls.push({ action: 'download', asset: value }),
      onRemove: (value) => calls.push({ action: 'remove', asset: value }),
    }));
    const trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    expect(trigger.getAttribute('aria-haspopup')).toBe('menu');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');

    for (const action of ['open', 'edit', 'download', 'remove'] as const) {
      const popup = await openMenu(trigger);
      fireEvent.click(within(popup).getByRole('menuitem', { name: labels[action] }));
      expect(calls[calls.length - 1]?.action).toBe(action);
      expect(calls[calls.length - 1]?.asset).toBe(asset);
      await expectClosed(trigger);
    }
    expect(calls.map((call) => call.action)).toEqual(['open', 'edit', 'download', 'remove']);
  });

  test('portals the popup outside the card so card overflow cannot clip actions', async () => {
    const portal = document.createElement('div');
    portal.id = 'creative-studio-portal-root';
    document.body.appendChild(portal);
    const { container, getByRole } = render(<article>{menu()}</article>);
    const popup = await openMenu(getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement);

    expect(container.querySelector('article')?.contains(popup)).toBe(false);
    expect(portal.contains(popup)).toBe(true);
  });

  test('omits unavailable callbacks and never offers a download for text assets', async () => {
    let edits = 0;
    let downloads = 0;
    const textAsset: CreativeAsset = { ...asset, kind: 'text', textContent: 'Reusable prompt' };
    const { getByRole } = render(menu({
      asset: textAsset,
      onOpen: undefined,
      onRemove: undefined,
      onEdit: () => { edits += 1; },
      onDownload: () => { downloads += 1; },
    }));
    const trigger = getByRole('button', { name: `更多：${textAsset.title}` }) as HTMLButtonElement;
    const popup = await openMenu(trigger);
    expect(within(popup).getAllByRole('menuitem').map((item) => item.textContent)).toEqual([labels.edit]);
    fireEvent.click(within(popup).getByRole('menuitem', { name: labels.edit }));
    expect(edits).toBe(1);
    expect(downloads).toBe(0);
    await expectClosed(trigger);
  });

  test('blocks disabled triggers and closes an open menu when actions become disabled', async () => {
    let calls = 0;
    const props = { onOpen: () => { calls += 1; } };
    const { getByRole, rerender } = render(menu({ ...props, disabled: true }));
    let trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    expect(trigger.disabled).toBe(true);
    fireEvent.click(trigger);
    expect(within(document.body).queryByRole('menu') === null).toBe(true);
    expect(calls).toBe(0);

    rerender(menu(props));
    trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    await openMenu(trigger);
    rerender(menu({ ...props, disabled: true }));
    trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    await expectClosed(trigger);
    expect(trigger.disabled).toBe(true);
    expect(calls).toBe(0);
  });

  test('supports arrow and boundary navigation, with Escape returning focus to the trigger', async () => {
    const { getByRole } = render(menu());
    const trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    const popup = await openMenu(trigger);
    const items = within(popup).getAllByRole('menuitem');
    await waitFor(() => expect(document.activeElement).toBe(items[0]));

    fireEvent.keyDown(items[0]!, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.keyDown(items[1]!, { key: 'End' });
    expect(document.activeElement).toBe(items[3]);
    fireEvent.keyDown(items[3]!, { key: 'Home' });
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(items[0]!, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(items[3]);
    fireEvent.keyDown(items[3]!, { key: 'Escape' });
    await expectClosed(trigger);
    expect(document.activeElement).toBe(trigger);
  });

  test('dismisses on Tab and outside interaction without invoking an action', async () => {
    let calls = 0;
    const { getByRole } = render(<>{menu({ onOpen: () => { calls += 1; } })}<button type='button'>Outside</button></>);
    const trigger = getByRole('button', { name: `更多：${asset.title}` }) as HTMLButtonElement;
    const popup = await openMenu(trigger);
    fireEvent.keyDown(within(popup).getAllByRole('menuitem')[0]!, { key: 'Tab' });
    await expectClosed(trigger);

    await openMenu(trigger);
    fireEvent.mouseDown(getByRole('button', { name: 'Outside' }));
    await expectClosed(trigger);
    expect(calls).toBe(0);
  });
});
