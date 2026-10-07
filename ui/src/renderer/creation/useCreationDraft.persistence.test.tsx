import '../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { parseConversationId, parseProviderId } from '@/common/types/ids';
import { initializeAgentBrowserStorageGeneration, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { emitter } from '@/renderer/utils/emitter';
import type { CreationDraft, CreationMode } from './types';
import { creationDraftStorageKey, emptyCreationDraft, removeCreationDraft, useCreationDraft, writeCreationDraft } from './useCreationDraft';

const datasetA = '0190f5fe-7c00-7a00-8000-000000000171';
const datasetB = '0190f5fe-7c00-7a00-8000-000000000172';
const conversationA = parseConversationId('0190f5fe-7c00-7a00-8000-000000000173');
const conversationB = parseConversationId('0190f5fe-7c00-7a00-8000-000000000174');
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000175');

function mediaDraft(mode: CreationMode): CreationDraft {
  const draft = emptyCreationDraft();
  return {
    ...draft,
    mode,
    lastMode: mode,
    models: { ...draft.models, [mode]: { providerId, model: `${mode}-chosen-model` } },
    parameters: { ...draft.parameters, [mode]: mode === 'music' ? { instrumental: false, seconds: 120 } : mode === 'video' ? { seconds: 10, aspect: '16:9' } : { count: 2, quality: 'high', aspect: '4:3' } },
    references: [{ asset_id: 'current-reference', kind: 'image', role: 'reference', title: '参考图' }],
    pendingPrompt: '继续这次创作',
    pendingFiles: ['/current/reference.png'],
  };
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setBrowserStorageGeneration(datasetA);
  initializeAgentBrowserStorageGeneration(1);
});

afterEach(() => {
  cleanup();
  mock.restore();
  localStorage.clear();
  sessionStorage.clear();
  initializeAgentBrowserStorageGeneration(1);
});

describe('historical conversation creation composer persistence', () => {
  test.each(['image', 'video', 'music'] as const)('%s selection and editable inputs survive ending the renderer session', mode => {
    const draft = mediaDraft(mode);
    const mounted = renderHook(() => useCreationDraft(conversationA));
    act(() => mounted.result.current.update(() => draft));
    expect(JSON.parse(localStorage.getItem(creationDraftStorageKey(conversationA))!)).toEqual(draft);
    mounted.unmount();

    // An application restart discards sessionStorage but retains localStorage.
    sessionStorage.clear();
    const reopened = renderHook(() => useCreationDraft(conversationA));
    expect(reopened.result.current.draft).toEqual(draft);
  });

  test('explicitly returning to chat survives restart while retaining the last media mode', () => {
    const draft = mediaDraft('video');
    const mounted = renderHook(() => useCreationDraft(conversationA));
    act(() => mounted.result.current.update(() => draft));
    act(() => mounted.result.current.setMode(null));
    mounted.unmount();
    sessionStorage.clear();

    const reopened = renderHook(() => useCreationDraft(conversationA));
    expect(reopened.result.current.draft).toEqual({ ...draft, mode: null });
    expect(reopened.result.current.draft.lastMode).toBe('video');
  });

  test('per-mode model choices and parameters remain independent after restart', () => {
    const draft = mediaDraft('image');
    draft.models.video = { providerId, model: 'video-choice' };
    draft.models.music = { providerId, model: 'music-choice' };
    draft.parameters.video = { seconds: 15, aspect: '16:9' };
    draft.parameters.music = { instrumental: true, seconds: 90 };
    writeCreationDraft(conversationA, draft);
    sessionStorage.clear();

    const reopened = renderHook(() => useCreationDraft(conversationA));
    for (const mode of ['video', 'music', 'image'] as const) {
      act(() => reopened.result.current.setMode(mode));
      expect(reopened.result.current.draft.models).toEqual(draft.models);
      expect(reopened.result.current.draft.parameters).toEqual(draft.parameters);
      expect(reopened.result.current.draft.references).toEqual(draft.references);
    }
  });

  test('switching conversation scopes restores each conversation without sharing edits', () => {
    writeCreationDraft(conversationA, mediaDraft('image'));
    writeCreationDraft(conversationB, mediaDraft('music'));
    const mounted = renderHook(({ scope }) => useCreationDraft(scope), { initialProps: { scope: conversationA } });
    const updatePreviousConversation = mounted.result.current.update;
    mounted.rerender({ scope: conversationB });
    expect(mounted.result.current.draft).toEqual(mediaDraft('music'));

    act(() => mounted.result.current.setMode('video'));
    const currentConversationDraft = mounted.result.current.draft;
    act(() => updatePreviousConversation(draft => ({ ...draft, pendingPrompt: '迟到的会话 A 编辑' })));
    expect(mounted.result.current.draft).toEqual(currentConversationDraft);
    expect(JSON.parse(localStorage.getItem(creationDraftStorageKey(conversationB))!)).toEqual(currentConversationDraft);

    mounted.rerender({ scope: conversationA });
    expect(mounted.result.current.draft.mode).toBe('image');
    expect(mounted.result.current.draft.models).toEqual(mediaDraft('image').models);
  });

  test('the same hook instance changes identity when the backend dataset generation changes', () => {
    const original = mediaDraft('image');
    writeCreationDraft(conversationA, original);
    const originalKey = creationDraftStorageKey(conversationA);
    const mounted = renderHook(() => useCreationDraft(conversationA));
    const staleUpdate = mounted.result.current.update;
    setBrowserStorageGeneration(datasetB);
    const restoredKey = creationDraftStorageKey(conversationA);
    mounted.rerender();

    expect(restoredKey).not.toBe(originalKey);
    expect(mounted.result.current.draft).toEqual(emptyCreationDraft());
    act(() => staleUpdate(() => mediaDraft('music')));
    expect(mounted.result.current.draft).toEqual(emptyCreationDraft());
    expect(JSON.parse(localStorage.getItem(originalKey)!)).toEqual(original);
    expect(localStorage.getItem(restoredKey)).toBeNull();
    act(() => mounted.result.current.setMode('video'));
    expect(JSON.parse(localStorage.getItem(originalKey)!)).toEqual(original);
    expect(JSON.parse(localStorage.getItem(restoredKey)!)).toMatchObject({ mode: 'video', lastMode: 'video' });

    setBrowserStorageGeneration(datasetA);
    mounted.rerender();
    expect(mounted.result.current.draft).toEqual(original);
  });

  test('the same hook instance does not reuse media drafts after an Agent-only clean cut', () => {
    const original = mediaDraft('music');
    writeCreationDraft(conversationA, original);
    const originalKey = creationDraftStorageKey(conversationA);
    const mounted = renderHook(() => useCreationDraft(conversationA));
    initializeAgentBrowserStorageGeneration(2);
    const cleanKey = creationDraftStorageKey(conversationA);
    mounted.rerender();

    expect(cleanKey).not.toBe(originalKey);
    expect(mounted.result.current.draft).toEqual(emptyCreationDraft());
    act(() => mounted.result.current.setMode('image'));
    expect(JSON.parse(localStorage.getItem(originalKey)!)).toEqual(original);
    expect(JSON.parse(localStorage.getItem(cleanKey)!)).toMatchObject({ mode: 'image' });
  });

  test('welcome-page drafts remain temporary and cannot read persistent conversation state', () => {
    const draft = mediaDraft('video');
    // A persistent value with this key cannot become a welcome-page draft.
    localStorage.setItem(creationDraftStorageKey('guid'), JSON.stringify(mediaDraft('music')));
    writeCreationDraft('guid', draft);
    expect(JSON.parse(sessionStorage.getItem(creationDraftStorageKey('guid'))!)).toEqual(draft);
    const mounted = renderHook(() => useCreationDraft('guid'));
    expect(mounted.result.current.draft).toEqual(draft);
    mounted.unmount();
    sessionStorage.clear();

    const reopened = renderHook(() => useCreationDraft('guid'));
    expect(reopened.result.current.draft).toEqual(emptyCreationDraft());
  });

  test.each(['guid', conversationA])('%s drafts discard cached Agent names and identities on read and write', scope => {
    const draft = mediaDraft('image');
    const identityMirrors = { agentLabel: 'removed label', presetId: 'removed-preset', selectedAgent: { kind: 'template', templateKey: 'creative-studio.default' } };
    const storage = scope === 'guid' ? sessionStorage : localStorage;
    const key = creationDraftStorageKey(scope);
    storage.setItem(key, JSON.stringify({ ...draft, ...identityMirrors }));
    const mounted = renderHook(() => useCreationDraft(scope));
    expect(mounted.result.current.draft).toEqual(draft);
    expect(JSON.parse(storage.getItem(key)!)).toEqual(draft);

    writeCreationDraft(scope, { ...draft, ...identityMirrors });
    expect(JSON.parse(storage.getItem(key)!)).toEqual(draft);
  });

  test.each([
    '{broken-json',
    'null',
    '17',
    JSON.stringify({ ...emptyCreationDraft(), mode: 'unsupported' }),
    JSON.stringify({ ...emptyCreationDraft(), lastMode: 'unsupported' }),
    JSON.stringify({ ...emptyCreationDraft(), models: null }),
    JSON.stringify({ ...emptyCreationDraft(), parameters: null }),
    JSON.stringify({ ...emptyCreationDraft(), references: {} }),
    JSON.stringify({ ...emptyCreationDraft(), references: [null] }),
    JSON.stringify({ ...emptyCreationDraft(), references: [17] }),
    JSON.stringify({ ...emptyCreationDraft(), references: [{ asset_id: 'asset', kind: 'image', role: 'reference' }] }),
    JSON.stringify({ ...emptyCreationDraft(), references: [{ asset_id: 'asset', kind: 'image', role: 'unknown', title: 'Image' }] }),
    JSON.stringify({ ...emptyCreationDraft(), references: [{ asset_id: 'asset', kind: 'unknown', role: 'reference', title: 'Image' }] }),
    JSON.stringify({ ...emptyCreationDraft(), references: [{ asset_id: 'asset', kind: 'image', role: 'reference', title: 'Image', url: 17 }] }),
  ])('invalid persistent draft does not prevent opening or editing the composer (%#)', saved => {
    localStorage.setItem(creationDraftStorageKey(conversationA), saved);
    const mounted = renderHook(() => useCreationDraft(conversationA));
    expect(mounted.result.current.draft).toEqual(emptyCreationDraft());
    act(() => mounted.result.current.setMode('video'));
    expect(mounted.result.current.draft.mode).toBe('video');
    expect(JSON.parse(localStorage.getItem(creationDraftStorageKey(conversationA))!)).toMatchObject({ mode: 'video' });
  });

  test('unavailable persistent storage keeps the current composer editable', () => {
    spyOn(localStorage, 'getItem').mockImplementation(() => { throw new Error('storage unavailable'); });
    spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('storage unavailable'); });
    const mounted = renderHook(() => useCreationDraft(conversationA));
    expect(mounted.result.current.draft).toEqual(emptyCreationDraft());
    act(() => mounted.result.current.setMode('music'));
    act(() => mounted.result.current.update(draft => ({ ...draft, parameters: { ...draft.parameters, music: { instrumental: false, seconds: 30 } } })));
    expect(mounted.result.current.draft).toMatchObject({ mode: 'music', lastMode: 'music', parameters: { music: { instrumental: false, seconds: 30 } } });
  });

  test.each(['image', 'video', 'music'] as const)('%s Guid-to-conversation handoff uses the durable draft writer', mode => {
    const draft = mediaDraft(mode);
    writeCreationDraft('guid', draft);
    const welcome = renderHook(() => useCreationDraft('guid'));
    const transferred = { ...welcome.result.current.draft, references: [], pendingFiles: [] };
    writeCreationDraft(conversationA, transferred);
    welcome.unmount();
    sessionStorage.clear();

    const history = renderHook(() => useCreationDraft(conversationA));
    expect(history.result.current.draft).toEqual(transferred);
    expect(sessionStorage.getItem(creationDraftStorageKey(conversationA))).toBeNull();
  });

  test('explicit draft removal clears only its current conversation', () => {
    writeCreationDraft(conversationA, mediaDraft('image'));
    writeCreationDraft(conversationB, mediaDraft('video'));
    removeCreationDraft(conversationA);
    expect(localStorage.getItem(creationDraftStorageKey(conversationA))).toBeNull();
    expect(JSON.parse(localStorage.getItem(creationDraftStorageKey(conversationB))!)).toEqual(mediaDraft('video'));
    const reopened = renderHook(() => useCreationDraft(conversationA));
    expect(reopened.result.current.draft).toEqual(emptyCreationDraft());
  });

  test('authoritative deletion clears a stored draft even when that conversation is unmounted', () => {
    writeCreationDraft(conversationA, mediaDraft('music'));
    writeCreationDraft(conversationB, mediaDraft('image'));
    emitter.emit('conversation.deleted', conversationA);
    expect(localStorage.getItem(creationDraftStorageKey(conversationA))).toBeNull();
    expect(JSON.parse(localStorage.getItem(creationDraftStorageKey(conversationB))!)).toEqual(mediaDraft('image'));
  });
});
