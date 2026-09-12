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
import type {
  AgentResourceSelection,
  AgentSessionCapabilitySelection,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  useGuidSend,
  type GuidSendDeps,
} from './useGuidSend';

const STORAGE_GENERATION = '0190f5fe-7c00-7a00-8000-000000000001';
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
  currentModel = MODEL,
  input = INPUT,
  loading = false,
  workspaceEnabled = true,
  resourceResolutionReady = true,
  resourceSelections = [],
  capabilitySelection,
  navigations = [],
}: {
  selection: GuidAgentSelection;
  selectedPreset?: ExecutableAgentPreset;
  currentModel?: TProviderWithModel | null;
  input?: string;
  loading?: boolean;
  workspaceEnabled?: boolean;
  resourceResolutionReady?: boolean;
  resourceSelections?: AgentResourceSelection[];
  capabilitySelection?: AgentSessionCapabilitySelection;
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
  current_model: currentModel ?? undefined,
  workspaceEnabled,
  resourceResolutionReady,
  resourceSelections,
  capabilitySelection,
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
    expect(calls[0]).toEqual({
      method: 'POST', url: '/api/agent-presets/from-template/chat.minimal',
      body: {
        display_name: 'agentSettings.template.chat.minimal.name',
        reuse_existing: true,
        model_route_refs: {},
        chat_route_records: {},
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
      },
    });
    expect(calls[1]).toEqual({
      method: 'POST',
      url: '/api/agent-sessions',
      body: {
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
        preset_id: PRESET_ID,
        title: INPUT,
      },
    });
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

  test('personal Agent launch freezes the selected session model without changing the preset', async () => {
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
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
        preset_id: PRESET_ID,
        title: INPUT,
      },
    });
    expect(Object.keys(calls[0].body as Record<string, unknown>).sort()).toEqual(
      ['model', 'preset_id', 'title']
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

  test('submits only product resource kind/id selections when creating the session', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const resourceSelections = [
      { resource_kind: 'knowledge_base', resource_id: '0190f5fe-7c00-7a00-8000-000000000201' },
      { resource_kind: 'workspace', resource_id: 'default-workspace' },
    ];
    const hook = renderHook(() => useGuidSend(createDeps({
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
      workspaceEnabled: false,
      resourceSelections,
    })));

    await act(async () => { await hook.result.current.handleSend(); });

    expect(calls[0].body).toEqual({
      model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
      preset_id: PRESET_ID,
      title: INPUT,
      resource_selections: resourceSelections,
    });
    expect(Object.keys((calls[0].body as { resource_selections: object[] }).resource_selections[0])).toEqual(['resource_kind', 'resource_id']);
  });

  test('freezes the selected Skills and MCP servers into the new session request', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const capabilitySelection = {
      enabled_skills: ['pdf'],
      excluded_auto_skills: ['cron'],
      mcp_server_ids: ['0190f5fe-7c00-7a00-8000-000000000202'],
    };
    const hook = renderHook(() => useGuidSend(createDeps({
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
      workspaceEnabled: false,
      capabilitySelection,
    })));

    await act(async () => { await hook.result.current.handleSend(); });

    expect(calls[0].body).toMatchObject({ capability_selection: capabilitySelection });
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

  test('preset mode blocks session creation until every required resource is selected', async () => {
    resetBrowserStorage();
    let calls = 0;
    globalThis.fetch = (async () => { calls++; throw new Error('must not fetch'); }) as typeof fetch;
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
    let failure: unknown;
    await act(async () => { try { await hook.result.current.handleSend(); } catch (error) { failure = error; } });
    expect((failure as Error).message).toBe('RESOURCE_SELECTION_REQUIRED');
    expect(calls).toBe(0);
  });

  test('requires both a workbench Agent and an explicit session model', () => {
    resetBrowserStorage();
    const templateReady = renderHook(() =>
      useGuidSend({
        ...createDeps({ selection: { kind: 'template', templateKey: 'chat.minimal' } }),
        selectedTemplate: TEMPLATE,
      })
    );
    const templateMissing = renderHook(() =>
      useGuidSend(
        createDeps({ selection: { kind: 'template', templateKey: 'chat.minimal' } })
      )
    );
    const templateMissingModel = renderHook(() =>
      useGuidSend({
        ...createDeps({
          selection: { kind: 'template', templateKey: 'chat.minimal' },
          currentModel: null,
        }),
        selectedTemplate: TEMPLATE,
      })
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
        })
      )
    );
    const emptyInput = renderHook(() =>
      useGuidSend(
        {
          ...createDeps({
            selection: { kind: 'template', templateKey: 'chat.minimal' },
            input: '   ',
          }),
          selectedTemplate: TEMPLATE,
        }
      )
    );

    expect(templateReady.result.current.isButtonDisabled).toBe(false);
    expect(templateMissing.result.current.isButtonDisabled).toBe(true);
    expect(templateMissingModel.result.current.isButtonDisabled).toBe(true);
    expect(presetReady.result.current.isButtonDisabled).toBe(false);
    expect(presetMissing.result.current.isButtonDisabled).toBe(true);
    expect(emptyInput.result.current.isButtonDisabled).toBe(true);
  });
});
