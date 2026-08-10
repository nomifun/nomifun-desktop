/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const viewer = readSource(new URL('./MiniAppViewer.tsx', import.meta.url));
const solidifyModal = readSource(new URL('./MiniAppSolidifyModal.tsx', import.meta.url));
const panel = readSource(new URL('../PreviewPanel/PreviewPanel.tsx', import.meta.url));

describe('MiniAppViewer structure', () => {
  test('renders the shared sandbox literal from the mini-app contract', () => {
    expect(viewer.includes("import { MINI_APP_IFRAME_SANDBOX } from '@renderer/pages/miniApps/contract'")).toBe(
      true
    );
    expect(viewer.includes('sandbox={MINI_APP_IFRAME_SANDBOX}')).toBe(true);
    // srcDoc, not a `src` to a file:// or backend URL — the preview renders the
    // in-memory body that the preview context keeps fresh.
    expect(viewer.includes('srcDoc={doc}')).toBe(true);
    expect(viewer.includes("sandbox='allow-scripts")).toBe(false);
  });

  test('the rendered document trails live content so an iterating agent cannot reset the app', () => {
    // Binding srcDoc straight to `content` reloads the iframe on every tick and
    // wipes the running app's state.
    expect(viewer.includes('srcDoc={content}')).toBe(false);
    expect(viewer.includes('const [doc, setDoc] = useState(content)')).toBe(true);
    expect(viewer.includes('MINI_APP_CONTENT_SETTLE_MS')).toBe(true);
    expect(viewer.includes('setTimeout(() => setDoc(content), MINI_APP_CONTENT_SETTLE_MS)')).toBe(true);
    expect(viewer.includes('clearTimeout(timer)')).toBe(true);
  });

  test('drops the HTML renderer typing animation and resource inlining', () => {
    expect(viewer.includes('useTypingAnimation')).toBe(false);
    expect(viewer.includes('inlineRelativeResources')).toBe(false);
    expect(viewer.includes('getImageBase64')).toBe(false);
  });

  test('refresh jumps to the newest body and remounts the iframe through a key bump', () => {
    expect(viewer.includes('const [refreshKey, setRefreshKey] = useState(0)')).toBe(true);
    expect(viewer.includes('setDoc(contentRef.current)')).toBe(true);
    expect(viewer.includes('setRefreshKey((prev) => prev + 1)')).toBe(true);
    expect(viewer.includes('key={`miniapp-${refreshKey}`}')).toBe(true);
  });

  test('toolbar rides the preview toolbar-extras portal with the shared button tokens', () => {
    expect(viewer.includes('usePreviewToolbarExtras')).toBe(true);
    expect(viewer.includes('toolbarExtrasContext.setExtras(')).toBe(true);
    expect(viewer.includes('toolbarExtrasContext.setExtras(null)')).toBe(true);
    expect(viewer.includes("t('miniApps.preview.title')")).toBe(true);
    expect(viewer.includes("t('miniApps.preview.refresh')")).toBe(true);
    expect(viewer.includes("t('miniApps.preview.solidify')")).toBe(true);

    // Style tokens come from PreviewToolbar, never pasted literals.
    expect(viewer.includes('PREVIEW_TOOLBAR_BTN_CLASS')).toBe(true);
    expect(viewer.includes('PREVIEW_TOOLBAR_BTN_ACTIVE_CLASS')).toBe(true);
    expect(viewer.includes('hover:text-t-primary hover:bg-3')).toBe(false);

    // The publish effect must not re-run on every content tick, so the solidify
    // callback reads the latest body out of a ref instead of closing over it.
    expect(viewer.includes('contentRef.current')).toBe(true);

    // Dead prop: nothing passed it, and the false path removed the only save
    // affordance the viewer has.
    expect(viewer.includes('hideToolbar')).toBe(false);
  });

  test('solidify reads the freshest file and goes through ipcBridge.miniapps', () => {
    expect(viewer.includes('ipcBridge.fs.readFile.invoke({ path: file_path, workspace })')).toBe(true);
    expect(viewer.includes('ipcBridge.miniapps.list.invoke()')).toBe(true);
    expect(viewer.includes('item.source_conversation_id === conversation_id')).toBe(true);
    expect(viewer.includes("t('miniApps.preview.readError')")).toBe(true);

    expect(solidifyModal.includes('ipcBridge.miniapps.update.invoke(')).toBe(true);
    expect(solidifyModal.includes('ipcBridge.miniapps.create.invoke(')).toBe(true);
    expect(solidifyModal.includes("t('miniApps.save.updateExisting'")).toBe(true);
    expect(solidifyModal.includes("t('miniApps.save.saveAsNew')")).toBe(true);
    expect(solidifyModal.includes("t('miniApps.save.nameRequired')")).toBe(true);
  });

  test('only the first solidify of a conversation records provenance', () => {
    // A fork must be unlinked: two rows sharing one `source_conversation_id`
    // would make the next "update the existing one" target an arbitrary row.
    expect(solidifyModal.includes('const isFork = existing !== null')).toBe(true);
    expect(
      solidifyModal.includes('...(conversation_id && !isFork ? { source_conversation_id: conversation_id } : {})')
    ).toBe(true);
  });

  test('panel dispatches the miniapp content type to the viewer', () => {
    expect(panel.includes("import MiniAppViewer from '../viewers/MiniAppViewer'")).toBe(true);
    expect(panel.includes("content_type === 'miniapp'")).toBe(true);
    expect(panel.includes('conversation_id={metadata?.conversation_id}')).toBe(true);
  });
});
