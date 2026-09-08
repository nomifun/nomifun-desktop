/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import type { TFunction } from 'i18next';
import type { Dispatch, SetStateAction } from 'react';
import type { NavigateFunction } from 'react-router-dom';

import type { TProviderWithModel } from '@/common/config/storage';
import {
  parseAgentPresetId,
  parseConversationId,
  parseProviderId,
} from '@/common/types/ids';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import type { ExecutableAgentPreset, GuidAgentSelection } from '../types';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import {
  useGuidSend,
  type GuidSendDeps,
} from './useGuidSend';

const STORAGE_GENERATION = '0190f5fe-7c00-7a00-8000-000000000001';
const DEFAULT_CONVERSATION_ID =
  '0190f5fe-7c00-7a00-8000-000000000101';
const PRESET_CONVERSATION_ID =
  '0190f5fe-7c00-7a00-8000-000000000102';
const AGENT_SESSION_ID = PRESET_CONVERSATION_ID;
const PRESET_ID = parseAgentPresetId(
  '0190f5fe-7c00-7a00-8000-000000000104'
);
const PROVIDER_ID = parseProviderId(
  '0190f5fe-7c00-7a00-8000-000000000105'
);
const INPUT = 'Review the release plan';
const FILES = ['C:\\workspace\\release.md', 'C:\\workspace\\risks.txt'];
const WORKSPACE = 'C:\\workspace';

const MODEL: TProviderWithModel = {
  id: PROVIDER_ID,
  platform: 'stepfun',
  name: 'StepFun',
  base_url: 'https://example.invalid/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  enabled: true,
  use_model: 'step-3.5-flash',
};

const PRESET = {
  preset_id: PRESET_ID,
  source: 'user',
  display_name: 'Release reviewer',
  bound_target_count: 0,
  current_stable_revision: {
    preset_id: PRESET_ID,
    revision: 3,
    revision_digest: 'a'.repeat(64),
  },
} as ExecutableAgentPreset;
const TEMPLATE = { template_key: 'chat.minimal' } as OfficialPresetTemplate;

type FetchCall = {
  method: string;
  url: string;
  body: unknown;
};

const realFetch = globalThis.fetch;

const jsonResponse = (data: unknown): Response =>
  new Response(JSON.stringify({ success: true, data }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });

const conversationProjection = (conversationId: string) => ({
  conversation_id: conversationId,
  name: INPUT,
  type: 'nomi',
  created_at: 1,
  modified_at: 2,
  extra: {
    workspace: WORKSPACE,
    custom_workspace: true,
  },
  model: {
    provider_id: PROVIDER_ID,
    model: MODEL.use_model,
  },
});

const installFetchRecorder = (): FetchCall[] => {
  const calls: FetchCall[] = [];
  globalThis.fetch = (async (
    input: string | URL | Request,
    init?: RequestInit
  ) => {
    const url = String(input);
    const method = init?.method ?? 'GET';
    const body =
      typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, url, body });

    if (method === 'POST' && url.endsWith('/api/agent-presets/from-template/chat.minimal')) {
      return jsonResponse({ preset: PRESET });
    }

    if (method === 'POST' && url.endsWith('/api/conversations')) {
      return jsonResponse(conversationProjection(DEFAULT_CONVERSATION_ID));
    }
    if (method === 'POST' && url.endsWith('/api/agent-sessions')) {
      return jsonResponse({ agent_session_id: AGENT_SESSION_ID });
    }
    if (
      method === 'GET' &&
      url.endsWith(`/api/conversations/${PRESET_CONVERSATION_ID}`)
    ) {
      return jsonResponse(conversationProjection(PRESET_CONVERSATION_ID));
    }
    if (
      method === 'PATCH' &&
      url.endsWith(`/api/conversations/${PRESET_CONVERSATION_ID}`)
    ) {
      return jsonResponse(conversationProjection(PRESET_CONVERSATION_ID));
    }

    throw new Error(`Unexpected request: ${method} ${url}`);
  }) as typeof fetch;
  return calls;
};

const noopDispatch = <T,>(): Dispatch<SetStateAction<T>> =>
  (() => undefined) as Dispatch<SetStateAction<T>>;

const createDeps = ({
  selection,
  selectedPreset,
  currentModel,
  input = INPUT,
  loading = false,
  workspaceEnabled = true,
  resourceResolutionReady = true,
  navigations = [],
}: {
  selection: GuidAgentSelection;
  selectedPreset?: ExecutableAgentPreset;
  currentModel?: TProviderWithModel;
  input?: string;
  loading?: boolean;
  workspaceEnabled?: boolean;
  resourceResolutionReady?: boolean;
  navigations?: string[];
}): GuidSendDeps => ({
  input,
  setInput: noopDispatch<string>(),
  files: FILES,
  setFiles: noopDispatch<string[]>(),
  dir: WORKSPACE,
  setDir: noopDispatch<string>(),
  setLoading: noopDispatch<boolean>(),
  loading,
  selection,
  selectedPreset,
  current_model: currentModel,
  workspaceEnabled,
  resourceResolutionReady,
  autoWork: { enabled: false },
  setMentionOpen: noopDispatch<boolean>(),
  setMentionQuery: noopDispatch<string | null>(),
  setMentionSelectorOpen: noopDispatch<boolean>(),
  setMentionActiveIndex: noopDispatch<number>(),
  navigate: ((target: string) => {
    navigations.push(target);
  }) as unknown as NavigateFunction,
  t: ((key: string) => key) as unknown as TFunction,
});

const readOnlyHandoff = () => {
  expect(sessionStorage.length).toBe(1);
  const key = sessionStorage.key(0);
  expect(key).not.toBeNull();
  const handoff = JSON.parse(sessionStorage.getItem(key!) ?? '{}') as Record<
    string,
    unknown
  >;
  expect(
    /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      String(handoff.idempotency_key)
    )
  ).toBe(true);
  return handoff;
};

const resetBrowserStorage = () => {
  setBrowserStorageGeneration(STORAGE_GENERATION);
  sessionStorage.clear();
};

afterEach(() => {
  cleanup();
  sessionStorage.clear();
  globalThis.fetch = realFetch;
});

describe('useGuidSend HTTP behavior', () => {
  test('official selection prepares its configuration only on send and launches a normal frozen session', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const navigations: string[] = [];
    const hook = renderHook(() => useGuidSend({
      ...createDeps({ selection: { kind: 'template', templateKey: 'chat.minimal' }, navigations, workspaceEnabled: false }),
      selectedTemplate: TEMPLATE,
    }));
    expect(calls).toHaveLength(0);
    expect(hook.result.current.isButtonDisabled).toBe(false);
    await act(async () => { await hook.result.current.handleSend(); });
    expect(calls[0]).toMatchObject({
      method: 'POST', url: '/api/agent-presets/from-template/chat.minimal',
      body: { reuse_existing: true, model_route_refs: {}, chat_route_records: {} },
    });
    expect(calls[1]).toEqual({ method: 'POST', url: '/api/agent-sessions', body: { preset_id: PRESET_ID, title: INPUT } });
    expect(calls).toHaveLength(3);
    expect(readOnlyHandoff()).toMatchObject({ input: INPUT, files: FILES });
    expect(navigations).toEqual([`/conversation/${PRESET_CONVERSATION_ID}`]);
  });

  test('failed official preparation does not create a session or navigate away', async () => {
    resetBrowserStorage();
    const navigations: string[] = [];
    let calls = 0;
    globalThis.fetch = (async () => {
      calls++;
      return new Response(JSON.stringify({ success: false, error: { code: 'CAPABILITY_NOT_MATERIALIZED', message: 'Missing capability' } }), { status: 422, headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    const hook = renderHook(() => useGuidSend({
      ...createDeps({ selection: { kind: 'template', templateKey: 'chat.minimal' }, navigations }),
      selectedTemplate: TEMPLATE,
    }));
    let failure: unknown;
    await act(async () => { try { await hook.result.current.handleSend(); } catch (error) { failure = error; } });
    expect(failure instanceof Error).toBe(true);
    expect(calls).toBe(1);
    expect(navigations).toEqual([]);
    expect(sessionStorage.length).toBe(0);
  });

  test('default mode POSTs the selected model, workspace and files, stages one handoff, and navigates', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const navigations: string[] = [];
    const hook = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'default' },
          currentModel: MODEL,
          navigations,
        })
      )
    );

    await act(async () => {
      await hook.result.current.handleSend();
    });

    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      method: 'POST',
      url: '/api/conversations',
      body: {
        type: 'nomi',
        name: INPUT,
        extra: {
          default_files: FILES,
          workspace: WORKSPACE,
          custom_workspace: true,
        },
        model: {
          provider_id: PROVIDER_ID,
          model: MODEL.use_model,
        },
      },
    });
    expect(readOnlyHandoff()).toMatchObject({
      conversation_id: parseConversationId(DEFAULT_CONVERSATION_ID),
      input: INPUT,
      files: FILES,
      initial_admission_epoch: 0,
    });
    expect(navigations).toEqual([
      `/conversation/${DEFAULT_CONVERSATION_ID}`,
    ]);
  });

  test('preset mode POSTs exactly preset_id/title, stages one handoff, and navigates', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const navigations: string[] = [];
    const hook = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'preset', presetId: PRESET_ID },
          selectedPreset: PRESET,
          navigations,
        })
      )
    );

    await act(async () => {
      await hook.result.current.handleSend();
    });

    expect(calls).toHaveLength(4);
    expect(calls[0]).toEqual({
      method: 'POST',
      url: '/api/agent-sessions',
      body: {
        preset_id: PRESET_ID,
        title: INPUT,
      },
    });
    expect(Object.keys(calls[0].body as Record<string, unknown>).sort()).toEqual(
      ['preset_id', 'title']
    );
    expect(calls[1]).toEqual({
      method: 'GET',
      url: `/api/conversations/${PRESET_CONVERSATION_ID}`,
      body: undefined,
    });
    expect(calls[2]).toEqual({
      method: 'PATCH',
      url: `/api/conversations/${PRESET_CONVERSATION_ID}`,
      body: {
        extra: {
          workspace: WORKSPACE,
        },
      },
    });
    expect(calls[3]).toEqual({
      method: 'GET',
      url: `/api/conversations/${PRESET_CONVERSATION_ID}`,
      body: undefined,
    });
    expect(readOnlyHandoff()).toMatchObject({
      conversation_id: parseConversationId(PRESET_CONVERSATION_ID),
      input: INPUT,
      files: FILES,
      initial_admission_epoch: 0,
    });
    expect(navigations).toEqual([
      `/conversation/${PRESET_CONVERSATION_ID}`,
    ]);
  });

  test('preset mode does not submit a workspace that the selected capabilities do not allow', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const navigations: string[] = [];
    const hook = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'preset', presetId: PRESET_ID },
          selectedPreset: PRESET,
          workspaceEnabled: false,
          navigations,
        })
      )
    );

    await act(async () => {
      await hook.result.current.handleSend();
    });

    expect(calls.map((call) => `${call.method} ${call.url}`)).toEqual([
      'POST /api/agent-sessions',
      `GET /api/conversations/${PRESET_CONVERSATION_ID}`,
    ]);
    expect(calls.some((call) => call.method === 'PATCH')).toBe(false);
    expect(navigations).toEqual([
      `/conversation/${PRESET_CONVERSATION_ID}`,
    ]);
  });

  test('preset mode remains disabled until exact capability resource resolution completes', () => {
    resetBrowserStorage();
    const hook = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'preset', presetId: PRESET_ID },
          selectedPreset: PRESET,
          resourceResolutionReady: false,
        })
      )
    );

    expect(hook.result.current.isButtonDisabled).toBe(true);
  });

  test('disables only when the active mode lacks its own launch target', () => {
    resetBrowserStorage();
    const defaultReady = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'default' },
          currentModel: MODEL,
        })
      )
    );
    const defaultMissingModel = renderHook(() =>
      useGuidSend(createDeps({ selection: { kind: 'default' } }))
    );
    const presetReady = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'preset', presetId: PRESET_ID },
          selectedPreset: PRESET,
        })
      )
    );
    const presetMissing = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'preset', presetId: PRESET_ID },
          currentModel: MODEL,
        })
      )
    );
    const emptyInput = renderHook(() =>
      useGuidSend(
        createDeps({
          selection: { kind: 'default' },
          currentModel: MODEL,
          input: '   ',
        })
      )
    );

    expect(defaultReady.result.current.isButtonDisabled).toBe(false);
    expect(defaultMissingModel.result.current.isButtonDisabled).toBe(true);
    expect(presetReady.result.current.isButtonDisabled).toBe(false);
    expect(presetMissing.result.current.isButtonDisabled).toBe(true);
    expect(emptyInput.result.current.isButtonDisabled).toBe(true);
  });
});
