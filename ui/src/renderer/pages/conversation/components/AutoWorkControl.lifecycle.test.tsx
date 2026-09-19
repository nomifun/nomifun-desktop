import '../../../../../test/setup-dom.ts';

import { Message } from '@arco-design/web-react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';

import { ipcBridge } from '@/common';
import type { IAutoWorkState } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseRequirementId } from '@/common/types/ids';
import AutoWorkControl from './AutoWorkControl';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });

const targetId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000027');
const requirementId = parseRequirementId('0190f5fe-7c00-7a00-8000-000000000028');
const restores: Array<() => void> = [];

afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

const state = (paused: boolean): IAutoWorkState => ({
  kind: 'conversation',
  target_id: targetId,
  enabled: true,
  tag: 'release',
  running: true,
  run_state: paused ? 'paused' : 'idle',
  paused,
  ...(paused ? { paused_reason: 'execution_failed' } : {}),
  ...(paused ? { current_requirement_id: undefined } : {}),
  completed_count: 0,
});

test('paused AutoWork reloads on reconnect/tag events and resumes from the conversation control', async () => {
  let paused = true;
  const getState = spyOn(ipcBridge.requirements.getAutoWork, 'invoke')
    .mockImplementation(async () => state(paused));
  const tags = spyOn(ipcBridge.requirements.tags, 'invoke').mockResolvedValue([{
    tag: 'release', pending: 1, in_progress: 0, done: 0, failed: 1,
    cancelled: 0, needs_review: 0, total: 2, paused: true,
    paused_reason: 'execution_failed',
  }]);
  const resume = spyOn(ipcBridge.requirements.resumeTag, 'invoke').mockImplementation(async () => {
    paused = false;
    return {
      tag: 'release', pending: 1, in_progress: 0, done: 0, failed: 1,
      cancelled: 0, needs_review: 0, total: 2, paused: false,
    };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => undefined as never);
  const error = spyOn(Message, 'error').mockImplementation(() => undefined as never);
  restores.push(
    () => getState.mockRestore(),
    () => tags.mockRestore(),
    () => resume.mockRestore(),
    () => success.mockRestore(),
    () => error.mockRestore(),
  );

  const listeners = new Map<string, Set<(event: any) => void>>();
  for (const [name, emitter] of Object.entries({
    created: ipcBridge.requirements.onCreated,
    updated: ipcBridge.requirements.onUpdated,
    status: ipcBridge.requirements.onStatusChanged,
    deleted: ipcBridge.requirements.onDeleted,
    paused: ipcBridge.requirements.onTagPaused,
    autowork: ipcBridge.requirements.onAutoWork,
    reconnect: ipcBridge.conversation.reconnected,
  })) {
    const subscription = spyOn(emitter, 'on').mockImplementation((listener: any) => {
      const set = listeners.get(name) ?? new Set();
      set.add(listener);
      listeners.set(name, set);
      return () => set.delete(listener);
    });
    restores.push(() => subscription.mockRestore());
  }

  const view = render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <AutoWorkControl target={{ kind: 'conversation', id: targetId }} />
      </I18nextProvider>
    </MemoryRouter>
  );
  await act(async () => {});
  expect(getState).toHaveBeenCalledTimes(1);

  await act(async () => {
    listeners.get('reconnect')?.forEach((listener) => listener(undefined));
  });
  expect(getState).toHaveBeenCalledTimes(2);

  await act(async () => {
    listeners.get('paused')?.forEach((listener) => listener({
      tag: 'release', reason: 'execution_failed', requirement_id: requirementId,
    }));
  });
  expect(getState).toHaveBeenCalledTimes(3);

  fireEvent.click(view.getByRole('button', { name: 'requirements.autowork.label' }));
  await act(async () => {});
  expect(document.body.textContent).toContain('requirements.autowork.state.paused');
  expect(document.body.textContent).toContain('requirements.autowork.pausedReasons.executionFailed');

  fireEvent.click(view.getByRole('button', { name: 'requirements.autowork.resume' }));
  await act(async () => {});
  expect(resume).toHaveBeenCalledWith({ tag: 'release', requeue_failed: true });
  expect(getState).toHaveBeenCalledTimes(4);
  expect(success).toHaveBeenCalledTimes(1);
});
