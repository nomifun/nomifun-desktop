/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import type { TFunction } from 'i18next';
import type { Dispatch, SetStateAction } from 'react';
import type { NavigateFunction } from 'react-router-dom';

import type { TProviderWithModel } from '@/common/config/storage';
import { ipcBridge } from '@/common';
import {
  conversationTarget,
  parseAgentPresetId,
  parseConversationId,
  parseProviderId,
  parseExecutionTemplateId,
} from '@/common/types/ids';
import { sessionStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { creationDraftStorageKey, emptyCreationDraft, useCreationDraft } from '@/renderer/creation/useCreationDraft';
import type { ExecutableAgentPreset, GuidAgentSelection } from '../types';
import type {
  AgentResourceSelection,
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
const EXECUTION_ID = '0190f5fe-7c00-7a00-8000-000000000107';
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
let workspaceMetadata: ReturnType<typeof spyOn> | undefined;

beforeEach(() => {
  workspaceMetadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({
    name: 'workspace',
    path: WORKSPACE,
    size: 0,
    type: 'inode/directory',
    lastModified: 1,
    isDirectory: true,
  });
});

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

test('home collaboration plan creates an execution bound to the frozen Session without queueing a normal turn', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const navigations: string[] = [];
  const collaboration: NonNullable<GuidSendDeps['collaboration']> = {
    execution_model_pool: { mode: 'range', models: [
      { provider_id: PROVIDER_ID, model: MODEL.use_model },
      { provider_id: PROVIDER_ID, model: 'reviewer' },
    ] },
    execution_template_id: parseExecutionTemplateId('0190f5fe-7c00-7a00-8000-000000000106'),
    delegation_policy: 'prefer_parallel', decision_policy: 'ask_user',
  };
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET, workspaceEnabled: false, navigations, files: [] }),
    collaboration,
  }));
  await act(async () => { await hook.result.current.handleSend(); });
  expect(calls.some(call => call.method === 'PATCH')).toBe(false);
  expect(calls.find(call => call.url.endsWith('/create-execution'))).toMatchObject({
    method: 'POST',
    url: `/api/agent-execution-templates/${collaboration.execution_template_id}/create-execution`,
    body: {
      goal: INPUT,
      work_dir: WORKSPACE,
      delegation_policy: 'prefer_parallel',
      decision_policy: 'ask_user',
      lead_conversation_id: PRESET_CONVERSATION_ID,
      lead_model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
    },
  });
  expect(navigations).toEqual([`/conversation/${PRESET_CONVERSATION_ID}`]);
  expect(sessionStorage.getItem(sessionStorageKey(
    'initial-message-nomi',
    conversationTarget(parseConversationId(PRESET_CONVERSATION_ID)),
  ))).toBeNull();
});

test('home collaboration model range creates the canonical execution request', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const navigations: string[] = [];
  const collaboration: NonNullable<GuidSendDeps['collaboration']> = {
    execution_model_pool: { mode: 'range', models: [
      { provider_id: PROVIDER_ID, model: MODEL.use_model },
      { provider_id: PROVIDER_ID, model: 'reviewer' },
    ] },
    execution_template_id: null,
    delegation_policy: 'prefer_parallel',
    decision_policy: 'ask_user',
  };
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET, workspaceEnabled: false, navigations, files: [] }),
    collaboration,
  }));

  await act(async () => { await hook.result.current.handleSend(); });

  expect(calls.find(call => call.url.endsWith('/api/agent-executions'))).toEqual({
    method: 'POST',
    url: '/api/agent-executions',
    body: {
      goal: INPUT,
      work_dir: WORKSPACE,
      model_pool: collaboration.execution_model_pool,
      delegation_policy: 'prefer_parallel',
      decision_policy: 'ask_user',
      lead_conversation_id: PRESET_CONVERSATION_ID,
      lead_model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
    },
  });
  expect(navigations).toEqual([`/conversation/${PRESET_CONVERSATION_ID}`]);
  expect(sessionStorage.length).toBe(0);
});

test('single-model automatic policy remains an ordinary initial conversation turn', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const navigations: string[] = [];
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET, workspaceEnabled: false, navigations, files: [] }),
    collaboration: {
      execution_model_pool: {
        mode: 'single',
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
      },
      execution_template_id: null,
      delegation_policy: 'automatic',
      decision_policy: 'automatic',
    },
  }));

  await act(async () => { await hook.result.current.handleSend(); });

  expect(calls.some((call) => call.url.includes('/api/agent-executions'))).toBe(false);
  expect(calls.some((call) => call.url.endsWith('/create-execution'))).toBe(false);
  expect(readOnlyHandoff().input).toBe(INPUT);
  expect(navigations).toEqual([`/conversation/${PRESET_CONVERSATION_ID}`]);
});

test('an explicit single-model decision policy starts a real execution', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const hook = renderHook(() => useGuidSend({
    ...createDeps({
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
      files: [],
    }),
    collaboration: {
      execution_model_pool: {
        mode: 'single',
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
      },
      execution_template_id: null,
      delegation_policy: 'automatic',
      decision_policy: 'ask_user',
    },
  }));

  await act(async () => { await hook.result.current.handleSend(); });

  expect(calls.find((call) => call.url.endsWith('/api/agent-executions'))?.body)
    .toMatchObject({ decision_policy: 'ask_user' });
  expect(sessionStorage.length).toBe(0);
});

test('failed collaboration admission deletes the empty Session and never navigates', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder({ failExecution: true });
  const navigations: string[] = [];
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET, workspaceEnabled: false, navigations, files: [] }),
    collaboration: {
      execution_model_pool: { mode: 'range', models: [
        { provider_id: PROVIDER_ID, model: MODEL.use_model },
        { provider_id: PROVIDER_ID, model: 'reviewer' },
      ] },
      execution_template_id: null,
      delegation_policy: 'prefer_parallel',
      decision_policy: 'automatic',
    },
  }));

  await expect(hook.result.current.handleSend()).rejects.toThrow('Execution admission failed');

  expect(calls.at(-1)).toMatchObject({
    method: 'DELETE',
    url: `/api/agent-sessions/${PRESET_CONVERSATION_ID}`,
  });
  expect(navigations).toEqual([]);
  expect(sessionStorage.length).toBe(0);
});

test('collaboration launch rejects attachments before creating a Session', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET }),
    collaboration: {
      execution_model_pool: { mode: 'range', models: [
        { provider_id: PROVIDER_ID, model: MODEL.use_model },
        { provider_id: PROVIDER_ID, model: 'reviewer' },
      ] },
      execution_template_id: null,
      delegation_policy: 'prefer_parallel',
      decision_policy: 'automatic',
    },
  }));

  await expect(hook.result.current.handleSend()).rejects.toThrow(
    'guid.collaboration.attachmentsUnsupported'
  );
  expect(calls).toHaveLength(0);
});

test('a failed advanced configuration step never hands off an unconfigured first message', async () => {
  resetBrowserStorage();
  const calls = installFetchRecorder();
  const navigations: string[] = [];
  const hook = renderHook(() => useGuidSend({
    ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET, workspaceEnabled: false, navigations }),
    applyAdvancedConfig: async () => { throw new Error('Cannot apply advanced settings'); },
  }));
  await expect(hook.result.current.handleSend()).rejects.toThrow();
  expect(navigations).toEqual([]);
  expect(calls.at(-1)).toMatchObject({
    method: 'DELETE',
    url: `/api/agent-sessions/${PRESET_CONVERSATION_ID}`,
  });
  expect(sessionStorage.getItem(sessionStorageKey('initial-message-nomi', conversationTarget(parseConversationId(PRESET_CONVERSATION_ID))))).toBeNull();
});

const installFetchRecorder = (options: { failExecution?: boolean } = {}): FetchCall[] => {
  const calls: FetchCall[] = [];
  let executionCommitted = false;
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
      url.endsWith(`/api/agent-sessions/${PRESET_CONVERSATION_ID}/projection`)
    ) {
      return jsonResponse({
        ...conversationProjection(PRESET_CONVERSATION_ID),
        ...(executionCommitted ? { linked_execution_id: EXECUTION_ID } : {}),
      });
    }
    if (
      method === 'POST' &&
      (url.endsWith('/api/agent-executions') || url.includes('/api/agent-execution-templates/')) &&
      (url.endsWith('/api/agent-executions') || url.endsWith('/create-execution'))
    ) {
      if (options.failExecution) {
        return new Response(JSON.stringify({
          success: false,
          error: { code: 'EXECUTION_ADMISSION_FAILED', message: 'Execution admission failed' },
        }), { status: 409, headers: { 'Content-Type': 'application/json' } });
      }
      executionCommitted = true;
      return jsonResponse({
        execution_id: EXECUTION_ID,
        goal: INPUT,
        lead_conversation_id: PRESET_CONVERSATION_ID,
        work_dir: WORKSPACE,
        delegation_policy: 'prefer_parallel',
        adaptation_policy: 'fixed',
        decision_policy: 'ask_user',
        max_parallel: 2,
        status: 'planning',
        summary: null,
        version: 0,
        plan_revision: 0,
        event_sequence: 1,
        created_at: 1,
        updated_at: 1,
      });
    }
    if (method === 'DELETE' && url.endsWith(`/api/agent-sessions/${PRESET_CONVERSATION_ID}`)) {
      return jsonResponse({
        agent_session_id: PRESET_CONVERSATION_ID,
        state: 'deleted',
        duplicate: false,
      });
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
  files = FILES,
  loading = false,
  workspaceEnabled = true,
  resourceResolutionReady = true,
  resourceSelections = [],
  knowledgePolicy,
  navigations = [],
}: {
  selection: GuidAgentSelection;
  selectedPreset?: ExecutableAgentPreset;
  currentModel?: TProviderWithModel | null;
  input?: string;
  files?: string[];
  loading?: boolean;
  workspaceEnabled?: boolean;
  resourceResolutionReady?: boolean;
  resourceSelections?: AgentResourceSelection[];
  knowledgePolicy?: GuidSendDeps['knowledgePolicy'];
  navigations?: string[];
}): GuidSendDeps => ({
  input,
  setInput: noopDispatch<string>(),
  files,
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
  knowledgePolicy,
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
  const key = sessionStorageKey('initial-message-nomi', conversationTarget(parseConversationId(PRESET_CONVERSATION_ID)));
  expect(sessionStorage.getItem(key)).not.toBeNull();
  const handoff = JSON.parse(sessionStorage.getItem(key) ?? '{}') as Record<
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
  workspaceMetadata?.mockRestore();
  workspaceMetadata = undefined;
});

describe('useGuidSend HTTP behavior', () => {
  test('does not expose a dedicated Browser-only AgentSession entry', () => {
    const hook = renderHook(() => useGuidSend(createDeps({
      input: '',
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
    })));
    expect(hook.result.current.isButtonDisabled).toBe(true);
    expect(hook.result.current).not.toHaveProperty('openBrowserHandler');
    expect(hook.result.current).not.toHaveProperty('isBrowserButtonDisabled');
  });

  test('launches the selected Agent without a composer-owned runtime override', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const hook = renderHook(() => useGuidSend({
      ...createDeps({ selection: { kind: 'preset', presetId: PRESET_ID }, selectedPreset: PRESET }),
    }));
    await act(async () => { await hook.result.current.handleSend(); });
    expect(calls[0].body).toMatchObject({ preset_id: PRESET_ID });
    expect(calls[0].body).not.toHaveProperty('runtime_build');
  });

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
    const nextTurn = renderHook(() => useCreationDraft(PRESET_CONVERSATION_ID));
    expect(nextTurn.result.current.draft.selectedAgent).toEqual({ kind: 'template', templateKey: 'chat.minimal' });
    expect(nextTurn.result.current.draft.presetId).toBe(PRESET_ID);
    expect(nextTurn.result.current.draft.mode).toBeNull();
  });

  test('official launch does not overwrite a draft already edited in the created conversation', async () => {
    resetBrowserStorage();
    installFetchRecorder();
    const existing = { ...emptyCreationDraft(), selectedAgent: { kind: 'preset', presetId: PRESET_ID }, pendingPrompt: 'Keep my newer draft', parameters: { image: { count: 2 }, video: {}, music: { instrumental: true } } };
    const key = creationDraftStorageKey(PRESET_CONVERSATION_ID);
    const hook = renderHook(() => useGuidSend({
      ...createDeps({ selection: { kind: 'template', templateKey: 'chat.minimal' }, workspaceEnabled: false }),
      selectedTemplate: TEMPLATE,
      applyAdvancedConfig: async () => { sessionStorage.setItem(key, JSON.stringify(existing)); },
    }));
    await act(async () => { await hook.result.current.handleSend(); });
    expect(JSON.parse(sessionStorage.getItem(key)!)).toEqual(existing);
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

    expect(calls).toHaveLength(2);
    expect(calls[0]).toEqual({
      method: 'POST',
      url: '/api/agent-sessions',
      body: {
        model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
        preset_id: PRESET_ID,
        title: INPUT,
        workspace: WORKSPACE,
      },
    });
    expect(Object.keys(calls[0].body as Record<string, unknown>).sort()).toEqual(
      ['model', 'preset_id', 'title', 'workspace']
    );
    expect(calls[1]).toEqual({
      method: 'GET',
      url: `/api/agent-sessions/${PRESET_CONVERSATION_ID}/projection`,
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
    const knowledgePolicy = {
      writeback: true,
      writeback_eagerness: 'auto' as const,
    };
    const hook = renderHook(() => useGuidSend(createDeps({
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
      workspaceEnabled: false,
      resourceSelections,
      knowledgePolicy,
    })));

    await act(async () => { await hook.result.current.handleSend(); });

    expect(calls[0].body).toEqual({
      model: { provider_id: PROVIDER_ID, model: MODEL.use_model },
      preset_id: PRESET_ID,
      title: INPUT,
      resource_selections: resourceSelections,
      knowledge_policy: knowledgePolicy,
    });
    expect(Object.keys((calls[0].body as { resource_selections: object[] }).resource_selections[0])).toEqual(['resource_kind', 'resource_id']);
  });

  test('minimal Agent creation relies on the saved binding without a session capability overlay', async () => {
    resetBrowserStorage();
    const calls = installFetchRecorder();
    const hook = renderHook(() => useGuidSend({
      ...createDeps({
        selection: { kind: 'template', templateKey: 'chat.minimal' },
        workspaceEnabled: false,
      }),
      selectedTemplate: TEMPLATE,
    }));

    await act(async () => { await hook.result.current.handleSend(); });

    expect(calls[1]).toMatchObject({ method: 'POST', url: '/api/agent-sessions' });
    expect(calls[1].body).not.toHaveProperty('capability_selection');
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
      `GET /api/agent-sessions/${PRESET_CONVERSATION_ID}/projection`,
    ]);
    expect(calls.some((call) => call.method === 'PATCH')).toBe(false);
    expect(navigations).toEqual([
      `/conversation/${PRESET_CONVERSATION_ID}`,
    ]);
  });

  test('rejects a removed project before creating an AgentSession', async () => {
    resetBrowserStorage();
    workspaceMetadata?.mockRejectedValueOnce(new Error('directory not found'));
    const calls = installFetchRecorder();
    const hook = renderHook(() => useGuidSend(createDeps({
      selection: { kind: 'preset', presetId: PRESET_ID },
      selectedPreset: PRESET,
    })));

    await expect(hook.result.current.handleSend()).rejects.toThrow('Workspace directory is unavailable');

    expect(calls).toHaveLength(0);
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
