/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { useEffect, useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { DelegationRecord } from '@/common/chat/delegationRecords';
import {
  parseAgentId, parseCompanionId, parseConversationId, parseExecutionAttemptId,
  parseExecutionId, parseExecutionParticipantId,
} from '@/common/types/ids';
import type { TAgentExecutionDetail, TExecutionParticipant } from '@/common/types/agentExecution/agentExecutionTypes';
import DelegationRecordRow from '@/renderer/pages/conversation/Messages/components/DelegationRecordRow';
import { ExecutionProvider, useExecution } from '@/renderer/pages/conversation/execution/ExecutionContext';
import ExecutionContentSwitcher from '@/renderer/pages/conversation/execution/ExecutionContentSwitcher';
import * as executionState from '@/renderer/pages/conversation/execution/useConversationExecution';
import * as readonlyTranscript from '@/renderer/pages/conversation/execution/ReadOnlyConversationView';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import {
  childConversationId, executionId, leadConversation, makeAttempt, makeDelegationRecord, makeDetail, makeStep,
} from '../../../../../test/fixtures/conversationDelegation';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { messages, agentExecution } } },
});
const restores: Array<() => void> = [];
beforeEach(() => {
  const activity = spyOn(ipcBridge.agentActivities.list, 'invoke').mockImplementation(async query => {
    if (!query.execution_id || !query.step_id || !query.attempt_id) throw new Error('Expected an exact task activity read');
    return { items: [{ key: `execution:${query.attempt_id}`, title: 'Selected task', kind: 'delegated_task', state: 'completed',
      agent_session_id: childConversationId, target: { kind: 'execution_task', execution_id: query.execution_id,
        step_id: query.step_id, attempt_id: query.attempt_id, expected_execution_version: 1, expected_step_version: 1 },
      continuation: 'none', waiting_reason: null, stop_accepted: false, execution_stopped: true,
      resources: [{ kind: 'runtime', state: 'released', detail: null }], can_stop: false, can_retry_cleanup: false, updated_at_ms: null,
    }], next_cursor: null };
  });
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(() => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  for (const spy of [activity, changed, reconnect]) restores.push(() => spy.mockRestore());
});
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach(restore => restore());
});

type MountCounter = { mounted: number; unmounted: number };

/** Keep the actual delegation button/context chain and a stateful lead composer. */
function CompanionMainConversation({ lifecycle, cardDetail, completed = false, batchRecords }: { lifecycle: MountCounter; cardDetail?: TAgentExecutionDetail; completed?: boolean; batchRecords?: DelegationRecord[] }) {
  const [draft, setDraft] = useState('');
  const execution = useExecution();
  const detail = cardDetail ?? execution.detail;
  useEffect(() => {
    lifecycle.mounted++;
    return () => { lifecycle.unmounted++; };
  }, [lifecycle]);
  return <section aria-label='Companion main conversation' data-testid='companion-main-conversation'>
    <textarea aria-label='Companion message draft' value={draft} onChange={event => setDraft(event.target.value)} />
    {batchRecords ? <DelegationRecordRow records={batchRecords} /> : detail?.steps.map(step => <DelegationRecordRow key={step.step_id} records={[makeDelegationRecord({
      id: `event:${step.step_id}`, execution_id: detail.execution.execution_id, step_id: step.step_id,
      title: step.title, kind: completed ? 'completed' : 'delegated',
      attempt_id: completed ? detail.attempts.find(attempt => attempt.step_id === step.step_id)?.attempt_id ?? null : null,
    })]} />)}
  </section>;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, decline) => { resolve = accept; reject = decline; });
  return { promise, resolve, reject };
}

/** Different rows own independent GETs while their shared main conversation remains mounted. */
function projectionRaceScene() {
  const slowStep = makeStep(20, { title: 'Slow A', status: 'completed' });
  const slowAttempt = makeAttempt(slowStep, { status: 'completed' });
  const slowDetail = makeDetail({ execution: { ...makeDetail().execution, status: 'completed' }, steps: [slowStep], attempts: [slowAttempt] });
  const fastId = parseExecutionId('00000000-07d0-7000-8000-000000000901');
  const fastChild = parseConversationId('00000000-09c4-7000-8000-000000000902');
  const fastStep = makeStep(21, { execution_id: fastId, title: 'Fast B', status: 'completed' });
  const fastAttempt = makeAttempt(fastStep, { execution_id: fastId, status: 'completed',
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000903'), conversation_id: fastChild });
  const fastDetail = makeDetail({ execution: { ...makeDetail().execution, execution_id: fastId, status: 'completed' }, steps: [fastStep], attempts: [fastAttempt] });
  const latestId = parseExecutionId('00000000-07d0-7000-8000-000000000904');
  const latest = makeDetail({ execution: { ...makeDetail().execution, execution_id: latestId, status: 'completed' } });
  const slow = deferred<TAgentExecutionDetail>();
  const refetch = mock(async () => {});
  const state = spyOn(executionState, 'useConversationExecution').mockReturnValue({
    executionId: latestId, detail: latest, refetch, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  const getExecution = spyOn(ipcBridge.agentExecution.get, 'invoke').mockImplementation(({ execution_id }) => {
    if (execution_id === executionId) return slow.promise;
    if (execution_id === fastId) return Promise.resolve(fastDetail);
    throw new Error('Unexpected execution in race');
  });
  const getChild = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => {
    const selected = conversation_id === childConversationId ? { step: slowStep, attempt: slowAttempt }
      : conversation_id === fastChild ? { step: fastStep, attempt: fastAttempt } : null;
    if (!selected) throw new Error('Unexpected child in race');
    return { ...leadConversation, id: conversation_id, name: selected.step.title, linked_execution_id: undefined,
      execution_step_id: selected.step.step_id, execution_attempt_id: selected.attempt.attempt_id };
  });
  const transcript = spyOn(readonlyTranscript, 'default').mockImplementation(({ conversation }) =>
    <section aria-label='Raced task transcript' data-session-id={conversation.id}>{conversation.name}</section>);
  for (const spy of [state, getExecution, getChild, transcript]) restores.push(() => spy.mockRestore());
  const records = [
    makeDelegationRecord({ id: 'race-a', title: slowStep.title, step_id: slowStep.step_id, kind: 'completed', attempt_id: slowAttempt.attempt_id }),
    makeDelegationRecord({ id: 'race-b', title: fastStep.title, execution_id: fastId, step_id: fastStep.step_id, kind: 'completed', attempt_id: fastAttempt.attempt_id }),
  ];
  let navigation!: ReturnType<typeof useExecution>;
  function Controls() {
    navigation = useExecution();
    return <aside>
      <button onClick={navigation.returnToMain}>Cancel task navigation</button>
      <button onClick={() => navigation.projectStep({ executionId: fastId, step: fastStep, attempt: fastAttempt,
        attemptId: fastAttempt.attempt_id, participants: [], refetch })}>Choose B synchronously</button>
    </aside>;
  }
  function Main() {
    const [draft, setDraft] = useState('');
    return <section data-testid='raced-main'>
      <textarea aria-label='Raced companion draft' value={draft} onChange={event => setDraft(event.target.value)} />
      {records.map(record => <DelegationRecordRow key={record.id} records={[record]} />)}
    </section>;
  }
  const scene = (id = leadConversation.id) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider conversation={{ ...leadConversation, id, linked_execution_id: latestId }}>
      <Controls /><ExecutionContentSwitcher><Main /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>;
  const view = render(scene());
  const chooseSlow = () => fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: slowStep.title }) }));
  const chooseFast = () => fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: fastStep.title }) }));
  return { view, scene, slow, slowDetail, fastChild, getChild, getExecution, chooseSlow, chooseFast, navigation: () => navigation };
}

test.each(['resolve', 'reject'] as const)('a stale slow row cannot replace a later fast exact projection or surface its late %s', async outcome => {
  const race = projectionRaceScene();
  const input = race.view.getByRole('textbox', { name: 'Raced companion draft' }) as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'Keep this draft' } });
  race.chooseSlow(); race.chooseFast();
  const selected = await race.view.findByRole('region', { name: 'Raced task transcript' });
  expect(selected.getAttribute('data-session-id')).toBe(race.fastChild);
  expect(race.view.getByTestId('raced-main').parentElement?.style.display).toBe('none');
  await act(async () => { if (outcome === 'resolve') race.slow.resolve(race.slowDetail); else race.slow.reject(new Error('Late obsolete read')); });
  expect(race.view.getByRole('region', { name: 'Raced task transcript' }).getAttribute('data-session-id')).toBe(race.fastChild);
  expect(race.getChild.mock.calls.map(([request]) => request.conversation_id)).toEqual([race.fastChild]);
  expect(race.view.container.textContent).not.toContain(messages.delegation.openFailed);
  fireEvent.click(race.view.getByRole('button', { name: 'Back to main conversation' }));
  expect(race.view.getByRole('textbox', { name: 'Raced companion draft' }) === input).toBe(true);
  expect(input.value).toBe('Keep this draft');
});

test('returning to main invalidates the latest unresolved row even before a projection opens', async () => {
  const race = projectionRaceScene();
  race.chooseSlow();
  fireEvent.click(race.view.getByRole('button', { name: 'Cancel task navigation' }));
  await act(async () => race.slow.resolve(race.slowDetail));
  expect(race.view.queryByRole('region', { name: 'Raced task transcript' })).toBeNull();
  expect(race.view.getByRole('textbox', { name: 'Raced companion draft' })).toBeTruthy();
  expect(race.getChild).not.toHaveBeenCalled();
});

test('a synchronous task selection also preempts an older async row lookup', async () => {
  const race = projectionRaceScene();
  race.chooseSlow();
  fireEvent.click(race.view.getByRole('button', { name: 'Choose B synchronously' }));
  await race.view.findByRole('region', { name: 'Raced task transcript' });
  await act(async () => race.slow.resolve(race.slowDetail));
  expect(race.view.getByRole('region', { name: 'Raced task transcript' }).getAttribute('data-session-id')).toBe(race.fastChild);
  expect(race.getChild.mock.calls.map(([request]) => request.conversation_id)).toEqual([race.fastChild]);
});

test('a Session switch invalidates an outstanding selection while its old source row stays mounted', async () => {
  const race = projectionRaceScene();
  race.chooseSlow();
  const nextSession = parseConversationId('00000000-09c4-7000-8000-000000000905');
  race.view.rerender(race.scene(nextSession));
  expect(race.navigation().conversationId).toBe(nextSession);
  await act(async () => race.slow.resolve(race.slowDetail));
  expect(race.navigation().projectedPayload).toBeNull();
  expect(race.getChild).not.toHaveBeenCalled();
});

test('unmount swallows a stale async rejection but an active row failure remains actionable', async () => {
  const race = projectionRaceScene();
  const stale = race.navigation().projectStepAsync(() => race.slow.promise.then(() => null));
  race.view.unmount();
  await act(async () => { race.slow.reject(new Error('Read after unmount')); await stale; });
  const active = projectionRaceScene();
  active.chooseSlow();
  await act(async () => active.slow.reject(new Error('Current lookup failed')));
  await waitFor(() => expect(active.view.container.textContent).toContain(messages.delegation.openFailed));
  expect(active.getChild).not.toHaveBeenCalled();
});

test('the companion surface mounts the shared content switcher around its conversation and resets projection by Session', async () => {
  const source = readFileSync(new URL('./CompanionCohabitView.tsx', import.meta.url), 'utf8');
  const provider = source.match(/<ExecutionProvider\b([^>]*)>([\s\S]*?)<\/ExecutionProvider>/);
  expect(provider).not.toBeNull();
  expect(provider![1]).toContain('key={conversation.id}');
  expect(provider![2]).toMatch(
    /<ExecutionContentSwitcher>\s*<CompanionConversation\b[\s\S]*?\/>\s*<\/ExecutionContentSwitcher>/,
  );
});

test.each([false, true])('opening a companion delegation batch %j displays the exact read-only attempt and preserves the mounted draft', async batch => {
  const step = makeStep(1, { title: 'Prepare a file on the local computer', status: 'completed' });
  const attempt = makeAttempt(step, { status: 'completed', finished_at: 3100 });
  const retryAttempt = makeAttempt(step, {
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000031'), attempt_no: 2,
    conversation_id: parseConversationId('00000000-09c4-7000-8000-000000000032'),
  });
  const otherStep = makeStep(2, { title: step.title, status: 'completed' });
  const otherAttempt = makeAttempt(otherStep, { attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000033'),
    conversation_id: parseConversationId('00000000-09c4-7000-8000-000000000034'), status: 'completed' });
  const detail = makeDetail({ steps: batch ? [{ ...step, status: 'running' }, otherStep] : [{ ...step, status: 'running' }],
    attempts: batch ? [attempt, retryAttempt, otherAttempt] : [attempt, retryAttempt] });
  const batchRecords = batch ? [
    makeDelegationRecord({ id: 'other-result', step_id: otherStep.step_id, title: otherStep.title, kind: 'completed', attempt_id: otherAttempt.attempt_id }),
    makeDelegationRecord({ id: 'old-result', step_id: step.step_id, title: step.title, kind: 'completed', attempt_id: attempt.attempt_id }),
  ] : undefined;
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
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} completed batchRecords={batchRecords} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  const input = view.getByRole('textbox', { name: 'Companion message draft' }) as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'A message drafted before opening the computer task' } });
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });

  if (batch) {
    expect(view.queryByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: step.title }) })).toBeNull();
    fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.toggleRecords', {
      action: i18n.t('messages.delegation.action.completed'), count: 2 }) }));
    expect(get).not.toHaveBeenCalled();
    fireEvent.click(view.container.querySelector<HTMLButtonElement>('[data-delegation-record-id="old-result"]')!);
  } else fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: step.title }) }));
  const projected = await view.findByRole('region', { name: 'Read-only task transcript' });
  expect(projected.getAttribute('data-session-id')).toBe(childConversationId);
  expect(projected.textContent).toContain('The local file was prepared.');
  expect(get).toHaveBeenCalledTimes(1);
  expect(get.mock.calls[0][0]).toEqual({ conversation_id: childConversationId });
  expect(transcript.mock.calls.at(-1)?.[0]).toMatchObject({ conversation: child, agent_name: step.title });
  expect(view.getByText(agentExecution.projection.history)).toBeTruthy();
  expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
  expect(view.getByRole('button', { name: agentExecution.projection.current })).toBeTruthy();
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
  fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: cardStep.title }) }));
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
  expect(getExecution).toHaveBeenCalledTimes(2);
  expect(getChild).toHaveBeenCalledTimes(1);
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });

  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(getExecution).toHaveBeenCalledTimes(3));
  expect(retry).toHaveBeenCalledTimes(1);
  expect(retry.mock.calls[0][0]).toEqual({
    execution_id: executionId,
    step_id: authoritativeStep.step_id,
    updates: { expected_execution_version: authoritative.execution.version, expected_step_version: authoritativeStep.version },
  });
  expect(getExecution.mock.calls.map(call => call[0])).toEqual([{ execution_id: executionId }, { execution_id: executionId }, { execution_id: executionId }]);
  expect(refreshCurrent).not.toHaveBeenCalled();
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
  expect(error).not.toHaveBeenCalled();

  fireEvent.click(view.getByRole('button', { name: 'Back to main conversation' }));
  expect(view.getByRole('textbox', { name: 'Companion message draft' })).toBe(input);
  expect(input.value).toBe('Keep this draft while inspecting the earlier task');
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });
  expect(view.queryByRole('region', { name: 'Read-only task transcript' })).toBeNull();
  expect(unsubscribeChanged).toHaveBeenCalledTimes(2);
  expect(unsubscribeReconnect).toHaveBeenCalledTimes(2);
});

test('an interrupted older task can be stopped from its projection without answering or cancelling the newer task', async () => {
  const step = makeStep(1, { status: 'waiting_input' });
  const attempt = makeAttempt(step, { status: 'waiting_input', question: 'Raw recovery diagnostic', runtime_state: {
    pending_conversation_effects: [], review_blocked: { kind: 'interrupted_turn', operation_id: 'exact-operation',
      receipt_state: 'running', reason: 'The exact prior operation result is unknown.' },
  } });
  const older = makeDetail({ steps: [step], attempts: [attempt] });
  older.execution = { ...older.execution, status: 'waiting_input', version: 23 };
  const newerId = parseExecutionId('00000000-07d0-7000-8000-000000000290');
  const newer = makeDetail({ execution: { ...older.execution, execution_id: newerId, version: 99 }, steps: [] });
  const child: TChatConversation = { ...leadConversation, id: childConversationId,
    execution_step_id: step.step_id, execution_attempt_id: attempt.attempt_id, linked_execution_id: undefined };
  const refreshNewer = mock(async () => {});
  const state = spyOn(executionState, 'useConversationExecution').mockReturnValue({
    executionId: newerId, detail: newer, refetch: refreshNewer, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  let authoritative = older;
  const getExecution = spyOn(ipcBridge.agentExecution.get, 'invoke').mockImplementation(async ({ execution_id }) => {
    if (execution_id !== executionId) throw new Error('Read the unrelated newer execution');
    return authoritative;
  });
  const getChild = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(child);
  const transcript = spyOn(readonlyTranscript, 'default').mockImplementation(() => <section aria-label='Task record'>Interrupted record</section>);
  const cancel = spyOn(ipcBridge.agentExecution.cancel, 'invoke').mockImplementation(async ({ execution_id }) => {
    if (execution_id !== executionId) throw new Error('Cancelled the unrelated newer execution');
    authoritative = { ...older, execution: { ...older.execution, status: 'cancelled', version: 24 },
      steps: [{ ...step, status: 'cancelled' }], attempts: [{ ...attempt, status: 'cancelled' }] };
    return authoritative;
  });
  const decision = spyOn(ipcBridge.agentExecution.answerDecision, 'invoke');
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(() => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const success = mock(() => () => {}), error = mock(() => () => {});
  const notices = spyOn(Message, 'useMessage').mockReturnValue([{ success, error }, <></>]);
  for (const spy of [state, getExecution, getChild, transcript, cancel, decision, changed, reconnect, notices]) restores.push(() => spy.mockRestore());
  const lifecycle: MountCounter = { mounted: 0, unmounted: 0 };
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider conversation={{ ...leadConversation, linked_execution_id: newerId }}>
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} cardDetail={older} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  const input = view.getByRole('textbox', { name: 'Companion message draft' }) as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'Preserve my next request' } });
  // The record stays compact; its exact inspector retains recovery and Stop.
  expect(view.queryByRole('button', { name: 'Stop collaboration' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: step.title }) }));
  await view.findByRole('region', { name: 'Task record' });
  expect(view.getAllByRole('status').some(element => element.textContent?.includes(agentExecution.recovery.title))).toBe(true);
  expect(view.queryByRole('button', { name: 'Submit decision' })).toBeNull();
  expect(view.queryByRole('textbox')).toBeNull();
  expect(view.getByText('The exact prior operation result is unknown.')).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Stop collaboration' }));
  await waitFor(() => expect(view.queryByRole('button', { name: 'Stop collaboration' })).toBeNull());
  expect(cancel.mock.calls[0][0]).toEqual({ execution_id: executionId, updates: { expected_version: 23 } });
  expect(getExecution.mock.calls.map(call => call[0])).toEqual([{ execution_id: executionId }, { execution_id: executionId }, { execution_id: executionId }]);
  expect(refreshNewer).not.toHaveBeenCalled();
  expect(decision).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
  expect(view.getByText(agentExecution.recovery.settledHint)).toBeTruthy();
  expect(view.getByRole('region', { name: 'Task record' })).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Back to main conversation' }));
  expect(view.getByRole('textbox', { name: 'Companion message draft' })).toBe(input);
  expect(input.value).toBe('Preserve my next request');
  expect(lifecycle).toEqual({ mounted: 1, unmounted: 0 });
});

for (const selection of ['root', 'completed_root', 'child'] as const) test(`an older AutoWork ${selection} projection keeps local and current-run Stop with their original owners`, async () => {
  const root = makeStep(0, { title: 'Original automatic root', status: selection === 'completed_root' ? 'completed' : 'running' });
  const childStep = makeStep(1, { title: 'Original automatic child' });
  const selected = selection === 'child' ? childStep : root;
  const rootAttempt = makeAttempt(root, { status: root.status === 'completed' ? 'completed' : 'running' });
  const childAttempt = makeAttempt(childStep, { attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000031') });
  const older = makeDetail({ steps: [root, childStep], attempts: [rootAttempt, childAttempt],
    automation_control: { root_step_id: root.step_id, lead_conversation_id: leadConversation.id,
      requirement_id: 'original-requirement', claim_generation: 7 } });
  const newerId = parseExecutionId('00000000-07d0-7000-8000-000000000390');
  const newer = makeDetail({ execution: { ...older.execution, execution_id: newerId, version: 99 }, steps: [] });
  const refreshNewer = mock(async () => {});
  const state = spyOn(executionState, 'useConversationExecution').mockReturnValue({
    executionId: newerId, detail: newer, refetch: refreshNewer, loading: false,
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false },
  });
  const getExecution = spyOn(ipcBridge.agentExecution.get, 'invoke').mockImplementation(async ({ execution_id }) => {
    if (execution_id !== executionId) throw new Error('Fetched the unrelated newer execution');
    return older;
  });
  const selectedAttempt = selection === 'child' ? childAttempt : rootAttempt;
  const getChild = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ ...leadConversation,
    id: childConversationId, execution_step_id: selected.step_id, execution_attempt_id: selectedAttempt.attempt_id, linked_execution_id: undefined });
  const transcript = spyOn(readonlyTranscript, 'default').mockImplementation(() => <section aria-label='Original task record' />);
  const cancelStep = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockResolvedValue(older);
  const cancelAll = spyOn(ipcBridge.agentExecution.cancel, 'invoke');
  const stopRun = spyOn(ipcBridge.agentActivities.stop, 'invoke').mockResolvedValue({ accepted: true, execution_stopped: false, resources: [] });
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(() => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const success = mock(() => () => {}), error = mock(() => () => {}), warning = mock(() => () => {});
  const notices = spyOn(Message, 'useMessage').mockReturnValue([{ success, error, warning }, <></>]);
  for (const spy of [state, getExecution, getChild, transcript, cancelStep, cancelAll, stopRun, changed, reconnect, notices]) restores.push(() => spy.mockRestore());
  const lifecycle: MountCounter = { mounted: 0, unmounted: 0 };
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ExecutionProvider conversation={{ ...leadConversation, linked_execution_id: newerId }}>
      <ExecutionContentSwitcher><CompanionMainConversation lifecycle={lifecycle} cardDetail={older} /></ExecutionContentSwitcher>
    </ExecutionProvider>
  </I18nextProvider></MemoryRouter>);
  expect(view.queryByRole('button', { name: 'Stop collaboration' })).toBeNull();
  expect(view.queryByRole('button', { name: 'Stop current automatic run' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: i18n.t('messages.delegation.openRecord', { title: selected.title }) }));
  await view.findByRole('region', { name: 'Original task record' });
  expect(view.queryByRole('button', { name: 'Retry' })).toBeNull();
  expect(view.queryByRole('button', { name: 'Adopt current result' })).toBeNull();
  if (selection === 'child') {
    expect(view.getByRole('button', { name: 'Stop task' })).toBeTruthy();
    fireEvent.click(view.getByRole('button', { name: 'Choose task stop scope' }));
    expect(await view.findByText('Stop task and its delegated work')).toBeTruthy();
    fireEvent.click(view.getByText('Stop only its delegated work'));
  } else {
    expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
    expect(view.queryByRole('button', { name: 'Choose task stop scope' })).toBeNull();
    fireEvent.click(view.getByRole('button', { name: 'Stop only its delegated work' }));
  }
  await waitFor(() => expect(cancelStep).toHaveBeenCalledTimes(1));
  expect(cancelStep.mock.calls[0][0]).toEqual({ execution_id: executionId, step_id: selected.step_id,
    updates: { expected_execution_version: older.execution.version, expected_step_version: selected.version, scope: 'delegated_work' } });
  fireEvent.click(view.getByRole('button', { name: 'Stop current automatic run' }));
  await waitFor(() => expect(stopRun).toHaveBeenCalledTimes(1));
  expect(stopRun.mock.calls[0][0]?.target).toEqual({ kind: 'auto_work_run', target_kind: 'conversation',
    target_id: leadConversation.id, requirement_id: 'original-requirement', claim_generation: 7 });
  expect(cancelAll).not.toHaveBeenCalled();
  expect(refreshNewer).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
});
