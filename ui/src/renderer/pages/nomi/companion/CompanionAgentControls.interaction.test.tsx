import '../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { Message } from '@arco-design/web-react';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { ipcBridge } from '@/common';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import type { PreviewAgentSessionSwitchResponse, ApplyAgentSessionSwitchResponse } from '@/common/types/agentPlatform';
import { asDigestHex } from '@/common/types/agentPlatform';
import type { TChatConversation } from '@/common/config/storage';
import nomi from '@/renderer/services/i18n/locales/en-US/nomi.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import CompanionAgentControls from './CompanionAgentControls';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { nomi, conversation, agentSettings, common } } },
  interpolation: { escapeValue: false },
});

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const sessionId = parseConversationId('019b0000-0000-7000-8000-000000000002');
const preview = {
  current: { label: 'companion.default', preset_id: 'companion-agent', preset_revision: 1, binding_version: 4 },
  target: { label: 'companion.default', preset_id: 'companion-agent', preset_revision: 2, binding_version: 5,
    resolved_snapshot_ref: { snapshot_id: 'target-snapshot', snapshot_digest: 'b'.repeat(64) },
  },
  model: { provider_id: 'provider', model: 'current-model', preserved: true, compatible: true, missing_features: [] },
  resources: { retained: [{ resource_kind: 'companion', resource_id: companionId }], dropped: [], missing_kinds: [] },
  capabilities: { gained: ['workspace.files'], lost: [] },
  handoff: { available: false, requirement_count: 0, verified_artifact_count: 0, unresolved_item_count: 0, completion_gate_inherited: false },
  blockers: [], expected_binding_version: 4, can_apply: true,
} as unknown as PreviewAgentSessionSwitchResponse;
const result = {
  conversation: { id: sessionId, type: 'nomi', extra: {}, agent_snapshot: { preset_revision: 2 } },
  transition_id: 'update-companion', binding_version: 5, effective_from: 'next_turn', warnings: [],
} as unknown as ApplyAgentSessionSwitchResponse<TChatConversation>;

afterEach(() => { cleanup(); mock.restore(); });

const mount = (existingSessionId?: typeof sessionId) => render(
  <MemoryRouter><I18nextProvider i18n={i18n}>
    <CompanionAgentControls companionId={companionId} sessionId={existingSessionId} surface='settings' />
  </I18nextProvider></MemoryRouter>
);

test('opens the official editor and applies a previewed update to the same Session only after confirmation', async () => {
  const current = spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockResolvedValue({ conversation_id: sessionId });
  const check = spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockResolvedValue(preview);
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke').mockResolvedValue(result);
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const view = mount();

  expect(view.getByRole('link', { name: 'Edit Agent' }).getAttribute('href')).toBe('/agent?template=companion.default');
  expect(apply).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  const panel = within(await view.findByTestId('companion-agent-update'));
  await waitFor(() => expect(panel.queryByText(/Workspace I\/O/)).toBeTruthy());
  expect(view.queryByRole('dialog')).toBeNull();
  expect(current).toHaveBeenCalledWith({ companion_id: companionId });
  expect(check).toHaveBeenCalledWith({ agent_session_id: sessionId, request: { selection: { kind: 'template', template_key: 'companion.default' } } });
  expect(apply).not.toHaveBeenCalled();
  expect(panel.getByText(nomi.chat.agentUpdateHint)).toBeTruthy();
  fireEvent.click(panel.getByRole('button', { name: nomi.chat.confirmAgentUpdates }));
  await waitFor(() => expect(success).toHaveBeenCalledWith(nomi.chat.agentUpdateSuccess));
  expect(apply.mock.calls[0][0]).toMatchObject({
    agent_session_id: sessionId,
    request: { selection: { kind: 'template', template_key: 'companion.default' }, handoff_mode: 'context_only', expected_binding_version: 4,
      expected_resolved_snapshot_ref: preview.target.resolved_snapshot_ref,
    },
  });
  expect(apply.mock.calls[0][0].request.model).toBeUndefined();
});

test('keeps a busy Session unchanged and shows its canonical blocker', async () => {
  const current = spyOn(ipcBridge.companion.getCompanionSession, 'invoke');
  spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockResolvedValue({ ...preview,
    can_apply: false, blockers: [{ code: 'AGENT_SESSION_TURN_ACTIVE', message: 'busy' }],
  });
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke');
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  await waitFor(() => expect(within(document.body).getByText(/Wait for the current response to finish/)).toBeTruthy());
  const confirm = view.getByRole('button', { name: nomi.chat.confirmAgentUpdates }) as HTMLButtonElement;
  expect(confirm.disabled).toBe(true);
  fireEvent.click(confirm);
  expect(current).not.toHaveBeenCalled();
  expect(apply).not.toHaveBeenCalled();
});

test('does not create a companion conversation just to apply a future default', async () => {
  spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockResolvedValue({ conversation_id: null });
  const ensure = spyOn(ipcBridge.companion.ensureCompanionSession, 'invoke');
  const check = spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke');
  const info = spyOn(Message, 'info').mockImplementation(() => () => {});
  const view = mount();
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  await waitFor(() => expect(info).toHaveBeenCalledWith(nomi.chat.agentNoSession));
  expect(ensure).not.toHaveBeenCalled();
  expect(check).not.toHaveBeenCalled();
});

test('a failed apply retains the exact preview and command key for an explicit retry', async () => {
  spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockResolvedValue(preview);
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke')
    .mockRejectedValueOnce(new Error('temporary disconnect')).mockResolvedValueOnce(result);
  spyOn(Message, 'success').mockImplementation(() => () => {});
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  await waitFor(() => expect(view.getByTestId('companion-agent-update').textContent).toContain('Workspace I/O'));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: nomi.chat.confirmAgentUpdates })); });
  await waitFor(() => expect(view.getByTestId('companion-agent-update').textContent).toContain('temporary disconnect'));
  fireEvent.click(view.getByRole('button', { name: nomi.chat.confirmAgentUpdates }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
  expect(apply.mock.calls[1][0]).toEqual(apply.mock.calls[0][0]);
});

test('a stale target disables the old confirmation and requires a fresh preview and command key', async () => {
  const refreshedPreview: PreviewAgentSessionSwitchResponse = { ...preview,
    target: { ...preview.target, preset_revision: 3,
      resolved_snapshot_ref: { ...preview.target.resolved_snapshot_ref, snapshot_digest: asDigestHex('c'.repeat(64)) },
    },
    capabilities: { gained: ['web.research'], lost: [] },
  };
  const check = spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke')
    .mockResolvedValueOnce(preview).mockResolvedValueOnce(refreshedPreview);
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke')
    .mockRejectedValueOnce(new BackendHttpError({ method: 'PUT', path: `/api/agent-sessions/${sessionId}/agent`, status: 409,
      body: { code: 'AGENT_SESSION_BINDING_CHANGED', error: 'The target changed after preview',
        details: { stage: 'target_preview', recovery: 'preview_again' },
      },
    })).mockResolvedValueOnce(result);
  spyOn(Message, 'success').mockImplementation(() => () => {});
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  await waitFor(() => expect(view.getByTestId('companion-agent-update').textContent).toContain('Workspace I/O'));
  const panel = () => within(view.getByTestId('companion-agent-update'));
  fireEvent.click(panel().getByRole('button', { name: nomi.chat.confirmAgentUpdates }));
  await waitFor(() => expect(panel().getByRole('button', { name: 'Preview updates again' })).toBeTruthy());
  const oldConfirm = panel().getByRole('button', { name: nomi.chat.confirmAgentUpdates }) as HTMLButtonElement;
  expect(oldConfirm.disabled).toBe(true);
  fireEvent.click(oldConfirm);
  expect(apply).toHaveBeenCalledTimes(1);
  fireEvent.click(panel().getByRole('button', { name: 'Preview updates again' }));
  await waitFor(() => expect(view.getByTestId('companion-agent-update').textContent).toContain('Web research'));
  expect(check).toHaveBeenCalledTimes(2);
  expect(apply).toHaveBeenCalledTimes(1);
  fireEvent.click(panel().getByRole('button', { name: nomi.chat.confirmAgentUpdates }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
  expect(apply.mock.calls[1][0].idempotency_key).not.toBe(apply.mock.calls[0][0].idempotency_key);
  expect(apply.mock.calls[1][0].request.expected_resolved_snapshot_ref).toEqual(refreshedPreview.target.resolved_snapshot_ref);
});

test('uses the available canonical task handoff automatically without a mode dialog', async () => {
  spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockResolvedValue({ ...preview,
    handoff: { ...preview.handoff, available: true },
  });
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke').mockResolvedValue(result);
  spyOn(Message, 'success').mockImplementation(() => () => {});
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  fireEvent.click(await view.findByRole('button', { name: nomi.chat.confirmAgentUpdates }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
  expect(apply.mock.calls[0][0].request.handoff_mode).toBe('continue_task');
  expect(view.queryByRole('dialog')).toBeNull();
});

test('closing an in-flight preview prevents its late response from reopening the update panel', async () => {
  let resolvePreview!: (value: PreviewAgentSessionSwitchResponse) => void;
  spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockImplementation(() => new Promise(resolve => { resolvePreview = resolve; }));
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke');
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  fireEvent.click(within(await view.findByTestId('companion-agent-update')).getByRole('button', { name: common.close }));
  await act(async () => { resolvePreview(preview); });
  expect(view.queryByTestId('companion-agent-update')).toBeNull();
  expect((view.getByRole('button', { name: nomi.chat.applyAgentUpdates }) as HTMLButtonElement).disabled).toBe(false);
  expect(apply).not.toHaveBeenCalled();
});

test('an already current conversation shows a brief informational message without an error panel', async () => {
  spyOn(ipcBridge.agentPlatform.sessions.previewAgentSwitch, 'invoke').mockRejectedValue(new BackendHttpError({ method: 'POST',
    path: `/api/agent-sessions/${sessionId}/agent-switch/preview`, status: 409,
    body: { code: 'AGENT_SESSION_AGENT_UNCHANGED', error: 'Already current' },
  }));
  const info = spyOn(Message, 'info').mockImplementation(() => () => {});
  const apply = spyOn(ipcBridge.agentPlatform.sessions.applyAgentSwitch, 'invoke');
  const view = mount(sessionId);
  fireEvent.click(view.getByRole('button', { name: nomi.chat.applyAgentUpdates }));
  await waitFor(() => expect(info).toHaveBeenCalledWith(nomi.chat.agentAlreadyCurrent));
  expect(view.queryByTestId('companion-agent-update')).toBeNull();
  expect(apply).not.toHaveBeenCalled();
});
