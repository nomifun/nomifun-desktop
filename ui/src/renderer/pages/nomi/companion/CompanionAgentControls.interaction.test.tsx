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
  fireEvent.click(view.getByRole('button', { name: 'Apply saved updates' }));
  const dialog = within(await view.findByTestId('agent-switch-dialog'));
  await waitFor(() => expect(dialog.queryByText('workspace.files')).toBeTruthy());
  expect(current).toHaveBeenCalledWith({ companion_id: companionId });
  expect(check).toHaveBeenCalledWith({ agent_session_id: sessionId, request: { selection: { kind: 'template', template_key: 'companion.default' } } });
  expect(apply).not.toHaveBeenCalled();
  expect(dialog.getByText('Keep the current model: current-model')).toBeTruthy();
  fireEvent.click(within(within(document.body).getByRole('dialog')).getByRole('button', { name: 'Apply saved updates' }));
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
  fireEvent.click(view.getByRole('button', { name: 'Apply saved updates' }));
  await waitFor(() => expect(within(document.body).getByText(/Wait for the current response to finish/)).toBeTruthy());
  const confirm = within(within(document.body).getByRole('dialog')).getByRole('button', { name: 'Apply saved updates' }) as HTMLButtonElement;
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
  fireEvent.click(view.getByRole('button', { name: 'Apply saved updates' }));
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
  fireEvent.click(view.getByRole('button', { name: 'Apply saved updates' }));
  await waitFor(() => expect(view.getByTestId('agent-switch-dialog').textContent).toContain('workspace.files'));
  await act(async () => { fireEvent.click(within(within(document.body).getByRole('dialog')).getByRole('button', { name: 'Apply saved updates' })); });
  await waitFor(() => expect(view.getByTestId('agent-switch-dialog').textContent).toContain('temporary disconnect'));
  fireEvent.click(within(within(document.body).getByRole('dialog')).getByRole('button', { name: 'Apply saved updates' }));
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
  fireEvent.click(view.getByRole('button', { name: 'Apply saved updates' }));
  await waitFor(() => expect(view.getByTestId('agent-switch-dialog').textContent).toContain('workspace.files'));
  const modal = () => within(within(document.body).getByRole('dialog'));
  fireEvent.click(modal().getByRole('button', { name: 'Apply saved updates' }));
  await waitFor(() => expect(modal().getByRole('button', { name: 'Preview updates again' })).toBeTruthy());
  const oldConfirm = modal().getByRole('button', { name: 'Apply saved updates' }) as HTMLButtonElement;
  expect(oldConfirm.disabled).toBe(true);
  fireEvent.click(oldConfirm);
  expect(apply).toHaveBeenCalledTimes(1);
  fireEvent.click(modal().getByRole('button', { name: 'Preview updates again' }));
  await waitFor(() => expect(view.getByTestId('agent-switch-dialog').textContent).toContain('web.research'));
  expect(check).toHaveBeenCalledTimes(2);
  expect(apply).toHaveBeenCalledTimes(1);
  fireEvent.click(modal().getByRole('button', { name: 'Apply saved updates' }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
  expect(apply.mock.calls[1][0].idempotency_key).not.toBe(apply.mock.calls[0][0].idempotency_key);
  expect(apply.mock.calls[1][0].request.expected_resolved_snapshot_ref).toEqual(refreshedPreview.target.resolved_snapshot_ref);
});
