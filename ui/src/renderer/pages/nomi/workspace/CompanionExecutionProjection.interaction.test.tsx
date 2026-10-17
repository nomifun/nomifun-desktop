/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { useEffect, useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import {
  parseAgentId, parseCompanionId, parseConversationId, parseExecutionAttemptId,
  parseExecutionId, parseExecutionParticipantId,
} from '@/common/types/ids';
import type { TAgentExecutionDetail, TExecutionParticipant } from '@/common/types/agentExecution/agentExecutionTypes';
import DelegationProgress from '@/renderer/pages/conversation/Messages/components/DelegationProgress';
import { ExecutionProvider, useExecution } from '@/renderer/pages/conversation/execution/ExecutionContext';
import ExecutionContentSwitcher from '@/renderer/pages/conversation/execution/ExecutionContentSwitcher';
import * as executionState from '@/renderer/pages/conversation/execution/useConversationExecution';
import * as readonlyTranscript from '@/renderer/pages/conversation/execution/ReadOnlyConversationView';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import {
  childConversationId, executionId, leadConversation, makeAttempt, makeDetail, makeStep, requestId,
} from '../../../../../test/fixtures/conversationDelegation';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { messages, agentExecution } } },
});
const restores: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach(restore => restore());
});

type MountCounter = { mounted: number; unmounted: number };

/** Keep the actual delegation button/context chain and a stateful lead composer. */
function CompanionMainConversation({ lifecycle, cardDetail }: { lifecycle: MountCounter; cardDetail?: TAgentExecutionDetail }) {
  const [draft, setDraft] = useState('');
  const execution = useExecution();
  const detail = cardDetail ?? execution.detail;
  useEffect(() => {
    lifecycle.mounted++;
    return () => { lifecycle.unmounted++; };
  }, [lifecycle]);
  return <section aria-label='Companion main conversation' data-testid='companion-main-conversation'>
    <textarea aria-label='Companion message draft' value={draft} onChange={event => setDraft(event.target.value)} />
    <DelegationProgress
      delegation={{ executionId: detail?.execution.execution_id ?? executionId, turnId: requestId, detail, unfinished: false }}
      projectStep={execution.projectStep}
      refetch={execution.refetch}
    />
  </section>;
}

test('the companion surface mounts the shared content switcher around its conversation and resets projection by Session', async () => {
  const source = readFileSync(new URL('./CompanionCohabitView.tsx', import.meta.url), 'utf8');
  const provider = source.match(/<ExecutionProvider\b([^>]*)>([\s\S]*?)<\/ExecutionProvider>/);
  expect(provider).not.toBeNull();
  expect(provider![1]).toContain('key={conversation.id}');
  expect(provider![2]).toMatch(
    /<ExecutionContentSwitcher>\s*<CompanionConversation\b[\s\S]*?\/>\s*<\/ExecutionContentSwitcher>/,
  );
});

test('opening a companion delegation displays the exact read-only attempt and returning preserves the mounted chat draft', async () => {
  const step = makeStep(1, { title: 'Prepare a file on the local computer', status: 'completed' });
  const attempt = makeAttempt(step, { status: 'completed', finished_at: 3100 });
  const detail = makeDetail({ steps: [step], attempts: [attempt] });
  const conversation: TChatConversation = {
    ...leadConversation,
    extra: {
      ...leadConversation.extra,
      companion_id: parseCompanionId('00000000-03e8-7000-8000-000000000006'),
      companion_role: 'robot',
    },
  };
  const child: TChatConversation = {
    ...leadConversation,
    id: childConversationId,
    name: 'Local computer task audit',
    linked_execution_id: undefined,
    execution_step_id: step.step_id,
    execution_attempt_id: attempt.attempt_id,
  };
  const refetch = mock(async () => {});
  const state = spyOn(executionState, 'useConversationExecution').mockReturnValue({
    executionId, detail, refetch, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => {
    if (conversation_id !== child.id) throw new Error('Unexpected projected Session');
    return child;
  });
  // Replace only the deepest chat renderer. Provider, projectStep, the switcher,
  // ProjectedAttemptView and its authoritative conversation fetch remain real.
  const transcript = spyOn(readonlyTranscript, 'default').mockImplementation(({ conversation: row, agent_name }) => (
    <section aria-label='Read-only task transcript' data-session-id={row.id}>
      <p>{agent_name}</p><p>The local file was prepared.</p>
    </section>
  ));
  for (const spy of [state, get, transcript]) restores.push(() => spy.mockRestore());

  const lifecycle: MountCounter = { mounted: 0, unmounted: 0 };
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider key={conversation.id} conversation={conversation}>
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  const input = view.getByRole('textbox', { name: 'Companion message draft' }) as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'A message drafted before opening the computer task' } });
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });

  fireEvent.click(view.getByRole('button', { name: `View subtask: ${step.title}` }));
  const projected = await view.findByRole('region', { name: 'Read-only task transcript' });
  expect(projected.getAttribute('data-session-id')).toBe(childConversationId);
  expect(projected.textContent).toContain('The local file was prepared.');
  expect(get).toHaveBeenCalledTimes(1);
  expect(get.mock.calls[0][0]).toEqual({ conversation_id: childConversationId });
  expect(transcript.mock.calls.at(-1)?.[0]).toMatchObject({ conversation: child, agent_name: step.title });
  expect(view.queryByRole('textbox', { name: 'Companion message draft' })).toBeNull();
  const main = view.getByTestId('companion-main-conversation');
  expect(main.parentElement?.style.display).toBe('none');
  expect(main.parentElement?.getAttribute('aria-hidden')).toBe('true');
  expect(main.querySelector('textarea')).toBe(input);
  expect(input.isConnected).toBe(true);
  expect(input.value).toBe('A message drafted before opening the computer task');
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });

  fireEvent.click(view.getByRole('button', { name: 'Back to main conversation' }));
  await waitFor(() => expect(view.queryByRole('region', { name: 'Read-only task transcript' })).toBeNull());
  expect(view.getByRole('textbox', { name: 'Companion message draft' })).toBe(input);
  expect(input.value).toBe('A message drafted before opening the computer task');
  expect(main.parentElement?.style.display).toBe('');
  expect(main.parentElement?.getAttribute('aria-hidden')).toBe('false');
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });
  expect(get).toHaveBeenCalledTimes(1);
  expect(refetch).not.toHaveBeenCalled();
});

test('an older message projects and retries its own execution while the provider observes a newer execution', async () => {
  const currentExecutionId = parseExecutionId('00000000-07d0-7000-8000-000000000090');
  const latestOldChildId = parseConversationId('00000000-09c4-7000-8000-000000000044');
  const currentChildId = parseConversationId('00000000-09c4-7000-8000-000000000094');
  const oldParticipant: TExecutionParticipant = {
    participant_id: parseExecutionParticipantId('00000000-09c4-7000-8000-000000000041'),
    execution_id: executionId,
    source_agent_id: parseAgentId('00000000-09c4-7000-8000-000000000042'),
    preset_id: null, preset_revision: null, agent_snapshot: null, provider_id: null,
    role: 'Earlier execution collaborator', model: 'earlier-model', capability: null,
    constraints: null, description: null, system_prompt: null, enabled_skills: [],
    disabled_builtin_skills: [], sort_order: 0, introduced_in_revision: 1,
    retired_in_revision: null, created_at: 2000,
  };
  const currentParticipant: TExecutionParticipant = {
    ...oldParticipant,
    participant_id: parseExecutionParticipantId('00000000-09c4-7000-8000-000000000091'),
    execution_id: currentExecutionId,
    source_agent_id: parseAgentId('00000000-09c4-7000-8000-000000000092'),
    role: 'New execution collaborator', model: 'new-model',
  };
  const cardStep = makeStep(2, {
    title: 'The earlier message computer task', status: 'completed', version: 2,
    assigned_participant_id: oldParticipant.participant_id,
  });
  const cardAttempt = makeAttempt(cardStep, {
    participant_id: oldParticipant.participant_id, status: 'completed', finished_at: 3100,
  });
  const card = makeDetail({ steps: [cardStep], attempts: [cardAttempt], participants: [oldParticipant] });
  card.execution = { ...card.execution, version: 3, status: 'completed' };
  const authoritativeStep = { ...cardStep, title: 'Authoritative earlier computer task', version: 6 };
  const authoritativeAttempt = {
    ...cardAttempt,
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000043'),
    attempt_no: 2, conversation_id: latestOldChildId,
  };
  const authoritative = makeDetail({
    execution: { ...card.execution, version: 11, event_sequence: 8 },
    steps: [authoritativeStep], attempts: [authoritativeAttempt],
    participants: [{ ...oldParticipant, role: 'Authoritative earlier collaborator' }],
  });
  const currentStep = makeStep(9, {
    execution_id: currentExecutionId, title: 'New execution task', status: 'completed', version: 86,
    assigned_participant_id: currentParticipant.participant_id,
  });
  const current = makeDetail({
    execution: { ...card.execution, execution_id: currentExecutionId, version: 91, event_sequence: 17 },
    steps: [currentStep], participants: [currentParticipant],
    attempts: [makeAttempt(currentStep, {
      execution_id: currentExecutionId, conversation_id: currentChildId,
      participant_id: currentParticipant.participant_id, status: 'completed',
    })],
  });
  const child: TChatConversation = {
    ...leadConversation, id: latestOldChildId, linked_execution_id: undefined,
    execution_step_id: authoritativeStep.step_id, execution_attempt_id: authoritativeAttempt.attempt_id,
  };
  const conversation = { ...leadConversation, linked_execution_id: currentExecutionId };
  const refreshCurrent = mock(async () => {});
  let resolveOldDetail!: (detail: TAgentExecutionDetail) => void;
  const pendingOldDetail = new Promise<TAgentExecutionDetail>(resolve => { resolveOldDetail = resolve; });
  let detailReads = 0;
  const getExecution = spyOn(ipcBridge.agentExecution.get, 'invoke').mockImplementation(({ execution_id }) => {
    if (execution_id !== executionId) return Promise.reject(new Error('Fetched the unrelated current execution'));
    return ++detailReads === 1 ? pendingOldDetail : Promise.resolve(authoritative);
  });
  const state = spyOn(executionState, 'useConversationExecution').mockReturnValue({
    executionId: currentExecutionId, detail: current, refetch: refreshCurrent, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  const getChild = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => {
    if (conversation_id !== latestOldChildId) throw new Error('Read an unrelated or superseded attempt Session');
    return child;
  });
  const transcript = spyOn(readonlyTranscript, 'default').mockImplementation(({ conversation: row, agent_name }) => (
    <section aria-label='Read-only task transcript' data-session-id={row.id}><p>{agent_name}</p></section>
  ));
  const retry = spyOn(ipcBridge.agentExecution.retry, 'invoke').mockResolvedValue(authoritative);
  const unsubscribeChanged = mock(() => {});
  const unsubscribeReconnect = mock(() => {});
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(unsubscribeChanged);
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(unsubscribeReconnect);
  const success = mock(() => () => {});
  const error = mock(() => () => {});
  const notices = spyOn(Message, 'useMessage').mockReturnValue([{ success, error }, <></>]);
  for (const spy of [getExecution, state, getChild, transcript, retry, changed, reconnect, notices]) {
    restores.push(() => spy.mockRestore());
  }

  const lifecycle: MountCounter = { mounted: 0, unmounted: 0 };
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider key={conversation.id} conversation={conversation}>
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} cardDetail={card} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  const input = view.getByRole('textbox', { name: 'Companion message draft' }) as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'Keep this draft while inspecting the earlier task' } });
  fireEvent.click(view.getByRole('button', { name: `View subtask: ${cardStep.title}` }));
  await waitFor(() => expect(getExecution).toHaveBeenCalledTimes(1));
  expect(getExecution.mock.calls[0][0]).toEqual({ execution_id: executionId });
  expect(getChild).not.toHaveBeenCalled();
  expect(transcript).not.toHaveBeenCalled();
  expect(view.queryByRole('button', { name: 'Retry' })).toBeNull();

  await act(async () => { resolveOldDetail(authoritative); });
  const projected = await view.findByRole('region', { name: 'Read-only task transcript' });
  expect(projected.getAttribute('data-session-id')).toBe(latestOldChildId);
  expect(projected.textContent).toContain(authoritativeStep.title);
  expect(projected.textContent).not.toContain(currentStep.title);
  expect(projected.textContent).not.toContain(currentParticipant.role!);
  expect(getChild).toHaveBeenCalledTimes(1);
  expect(getChild.mock.calls[0][0]).toEqual({ conversation_id: latestOldChildId });
  expect(transcript.mock.calls.at(-1)?.[0]).toMatchObject({ conversation: child, agent_name: authoritativeStep.title });

  const nextExecutionId = parseExecutionId('00000000-07d0-7000-8000-000000000190');
  state.mockReturnValue({
    executionId: nextExecutionId,
    detail: { ...current, execution: { ...current.execution, execution_id: nextExecutionId, version: 173 },
      steps: [], attempts: [], participants: [] },
    refetch: refreshCurrent, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  view.rerender(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider key={conversation.id} conversation={{ ...conversation, linked_execution_id: nextExecutionId }}>
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} cardDetail={card} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  expect(view.getByRole('region', { name: 'Read-only task transcript' }).getAttribute('data-session-id')).toBe(latestOldChildId);
  expect(view.queryByRole('textbox', { name: 'Companion message draft' })).toBeNull();
  expect(getExecution).toHaveBeenCalledTimes(1);
  expect(getChild).toHaveBeenCalledTimes(1);
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });

  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(getExecution).toHaveBeenCalledTimes(2));
  expect(retry).toHaveBeenCalledTimes(1);
  expect(retry.mock.calls[0][0]).toEqual({
    execution_id: executionId,
    step_id: authoritativeStep.step_id,
    updates: { expected_execution_version: authoritative.execution.version, expected_step_version: authoritativeStep.version },
  });
  expect(getExecution.mock.calls.map(call => call[0])).toEqual([{ execution_id: executionId }, { execution_id: executionId }]);
  expect(refreshCurrent).not.toHaveBeenCalled();
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
  expect(error).not.toHaveBeenCalled();

  fireEvent.click(view.getByRole('button', { name: 'Back to main conversation' }));
  expect(view.getByRole('textbox', { name: 'Companion message draft' })).toBe(input);
  expect(input.value).toBe('Keep this draft while inspecting the earlier task');
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });
  expect(view.queryByRole('region', { name: 'Read-only task transcript' })).toBeNull();
  expect(unsubscribeChanged).toHaveBeenCalledTimes(1);
  expect(unsubscribeReconnect).toHaveBeenCalledTimes(1);
});
