/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const guidPage = readSource(new URL('./GuidPage.tsx', import.meta.url));
const entryStrip = readSource(new URL('./components/ComposerEntryStrip.tsx', import.meta.url));
const quickStart = readSource(new URL('../../hooks/agent/useMiniAppQuickStart.ts', import.meta.url));
const nomiQuickStart = readSource(new URL('../../hooks/agent/useNomiQuickStart.ts', import.meta.url));

describe('mini-app composer entry wiring', () => {
  test('entry strip exposes the entry, the dismissible active token, and its i18n keys', () => {
    expect(entryStrip.includes("t('miniApps.composer.entry')")).toBe(true);
    expect(entryStrip.includes("t('miniApps.composer.activeLabel')")).toBe(true);
    expect(entryStrip.includes("t('miniApps.composer.dismiss')")).toBe(true);
    expect(entryStrip.includes('onCreateMiniApp')).toBe(true);
    expect(entryStrip.includes('miniAppActive')).toBe(true);
    expect(entryStrip.includes('onDismissMiniApp')).toBe(true);
    // Active state reuses the preset persona token styling.
    expect(entryStrip.includes('styles.entryButtonActive')).toBe(true);
    expect(entryStrip.includes('styles.entryDismiss')).toBe(true);
    // IconPark: plain named import, never aliased or namespaced.
    expect(entryStrip.includes("import { ApplicationOne, EveryUser, Lightning, Robot } from '@icon-park/react'")).toBe(
      true
    );
  });

  test('GuidPage owns the mode, swaps the placeholder, and routes the send', () => {
    expect(guidPage.includes('const [miniAppMode, setMiniAppMode] = useState(false)')).toBe(true);
    expect(guidPage.includes('miniAppActive={miniAppMode}')).toBe(true);
    expect(guidPage.includes('onDismissMiniApp={() => setMiniAppMode(false)}')).toBe(true);
    expect(guidPage.includes("t('miniApps.composer.placeholder')")).toBe(true);

    // The mini-app branch must bypass useGuidSend entirely, and both submit
    // gestures (Enter + the send button) must go through the same router.
    expect(guidPage.includes('const handleComposerSend = useCallback(')).toBe(true);
    expect(guidPage.includes('onSend={handleComposerSend}')).toBe(true);
    expect(guidPage.includes('handleComposerSend();')).toBe(true);
    expect(guidPage.includes('onSend={send.sendMessageHandler}')).toBe(false);
    expect(guidPage.includes('miniAppQuickStart\n      .start({')).toBe(true);
    expect(guidPage.includes('setMiniAppMode(false);')).toBe(true);
  });

  test('the send is guarded, carries the staged inputs, and resets the composer', () => {
    // A synchronous ref guard, not just the `loading` state: two gestures in one
    // tick would otherwise create two conversations.
    expect(guidPage.includes('const miniAppSendingRef = useRef(false)')).toBe(true);
    expect(guidPage.includes('|| miniAppSendingRef.current) return')).toBe(true);
    expect(guidPage.includes('miniAppSendingRef.current = true')).toBe(true);
    expect(guidPage.includes('miniAppSendingRef.current = false')).toBe(true);

    // The picker GuidPage owns, plus the staged workspace + attachments.
    expect(guidPage.includes('model: modelSelection.current_model')).toBe(true);
    expect(guidPage.includes('dir: guidInput.dir')).toBe(true);
    expect(guidPage.includes('files: guidInput.files')).toBe(true);

    // Same teardown as the normal path.
    expect(guidPage.includes('guidInput.setFiles([]);')).toBe(true);
    expect(guidPage.includes('guidInput.setDir(\'\');')).toBe(true);
    expect(guidPage.includes('mention.setMentionSelectorOpen(false);')).toBe(true);

    // Identity churn: the handler depends on the memoized `.start`, not the hook
    // object that would be recreated on every render.
    expect(guidPage.includes('miniAppQuickStart.start,')).toBe(true);
  });

  test('mini-app mode and an armed AutoWork entry are mutually exclusive', () => {
    expect(guidPage.includes('if (isAutoWorkMode) setMiniAppMode(false);')).toBe(true);
    expect(guidPage.includes('onCreateMiniApp={isAutoWorkMode ? undefined : () => setMiniAppMode(true)}')).toBe(true);
  });

  test('GuidPage activates from the ?miniapp=1 query and strips it', () => {
    expect(guidPage.includes("new URLSearchParams(location.search).get('miniapp') === '1'")).toBe(true);
    expect(guidPage.includes('navigate(`${location.pathname}${location.hash}`, { replace: true, state: null })')).toBe(
      true
    );
    // No "already handled" ref: stripping the query is what prevents re-entry,
    // and a ref would also block a fresh same-route activation.
    expect(guidPage.includes('miniAppQueryHandledRef')).toBe(false);
  });

  test('the mini-app quick start is a thin wrapper over the Nomi one', () => {
    expect(quickStart.includes('useNomiQuickStart')).toBe(true);
    expect(quickStart.includes('startNomi({')).toBe(true);
    expect(quickStart.includes('system_prompt: MINI_APP_BUILDER_SYSTEM_PROMPT')).toBe(true);
    expect(quickStart.includes('[MINI_APP_EXTRA_FLAG]: true')).toBe(true);
    expect(quickStart.includes("t('miniApps.composer.conversationName'")).toBe(true);
    expect(quickStart.includes('MINI_APP_NAME_SNIPPET_LENGTH')).toBe(true);
    // Write-only marker: nothing ever read `miniapp_file`.
    expect(quickStart.includes('MINI_APP_EXTRA_FILE')).toBe(false);
    // The create / sessionStorage / navigate sequence lives in ONE place.
    expect(quickStart.includes('ipcBridge.conversation.create')).toBe(false);
    expect(quickStart.includes('sessionStorage')).toBe(false);
    expect(quickStart.includes('useNavigate')).toBe(false);
    // Stable return object so callers can depend on `.start` alone.
    expect(quickStart.includes('useMemo(() => ({ start, canStart })')).toBe(true);
  });

  test('the shared Nomi quick start owns the handoff protocol and the model override', () => {
    expect(nomiQuickStart.includes("type: 'nomi'")).toBe(true);
    expect(nomiQuickStart.includes('const effectiveModel = model ?? current_model')).toBe(true);
    expect(nomiQuickStart.includes('...extra,')).toBe(true);
    expect(nomiQuickStart.includes("sessionStorageKey('initial-message-nomi'")).toBe(true);
    expect(nomiQuickStart.includes("emitter.emit('chat.history.refresh')")).toBe(true);
    expect(nomiQuickStart.includes('seedConversationCache(conversation)')).toBe(true);
    expect(nomiQuickStart.includes('navigate(`/conversation/${conversation.id}`)')).toBe(true);
    // Create failures report the backend's own reason (e.g. a rejected workspace
    // path) instead of a generic "create failed".
    expect(nomiQuickStart.includes('getConversationCreateErrorMessage(error, t)')).toBe(true);
  });
});
