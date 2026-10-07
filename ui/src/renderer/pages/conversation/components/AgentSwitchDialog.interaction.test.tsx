import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { PreviewAgentSessionSwitchResponse } from '@/common/types/agentPlatform';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import AgentSwitchDialog from './AgentSwitchDialog';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { conversation, common } } },
  interpolation: { escapeValue: false },
});

const preview = {
  current: {
    label: 'Research Agent', preset_id: 'source', preset_revision: 3,
    resolved_snapshot_ref: { snapshot_id: 'source', snapshot_digest: 'a'.repeat(64) },
    binding_version: 4,
  },
  target: {
    label: 'Coding Agent', preset_id: 'target', preset_revision: 7,
    resolved_snapshot_ref: { snapshot_id: 'target', snapshot_digest: 'b'.repeat(64) },
    binding_version: 5,
  },
  model: {
    provider_id: 'provider', model: 'model', preserved: true, compatible: true,
    missing_features: [],
  },
  resources: {
    retained: [{ resource_kind: 'workspace', resource_id: 'workspace' }],
    dropped: [{ resource_kind: 'browser', resource_id: 'browser' }],
    missing_kinds: [],
  },
  capabilities: { gained: ['workspace.files'], lost: ['web.research'] },
  handoff: {
    available: true, requirement_count: 2, verified_artifact_count: 1,
    unresolved_item_count: 1, completion_gate_inherited: false,
  },
  blockers: [], expected_binding_version: 4, can_apply: true,
} as unknown as PreviewAgentSessionSwitchResponse;

afterEach(() => { cleanup(); mock.restore(); });

describe('AgentSwitchDialog', () => {
  test('makes the next-Turn boundary and data-only handoff choice explicit', () => {
    const onModeChange = mock(() => {});
    const onConfirm = mock(() => {});
    render(
      <I18nextProvider i18n={i18n}>
        <AgentSwitchDialog
          visible
          preview={preview}
          loading={false}
          applying={false}
          mode='continue_task'
          onModeChange={onModeChange}
          onConfirm={onConfirm}
          onCancel={() => {}}
          onRecovery={() => {}}
        />
      </I18nextProvider>
    );
    const dialog = within(document.body).getByTestId('agent-switch-dialog');
    expect(dialog.textContent).toContain('Research Agent');
    expect(dialog.textContent).toContain('Coding Agent');
    expect(dialog.textContent).toContain('next message');
    expect(dialog.textContent).toContain('Old requirements do not enter');
    expect(dialog.textContent).toContain('workspace.files');
    expect(dialog.textContent).toContain('web.research');
    fireEvent.click(within(document.body).getByText('Chat context only'));
    expect(onModeChange).toHaveBeenCalledWith('context_only');
    fireEvent.click(within(document.body).getByRole('button', { name: 'Switch Agent' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  test('shows the localized template name supplied by the current conversation', () => {
    render(
      <I18nextProvider i18n={i18n}>
        <AgentSwitchDialog
          visible preview={preview} currentAgentLabel='General' targetAgentLabel='Minimal'
          loading={false} applying={false} mode='context_only'
          onModeChange={() => {}} onConfirm={() => {}} onCancel={() => {}} onRecovery={() => {}}
        />
      </I18nextProvider>
    );
    const dialog = within(document.body).getByTestId('agent-switch-dialog');
    expect(dialog.textContent).toContain('General');
    expect(dialog.textContent).toContain('Minimal');
    expect(dialog.textContent).not.toContain('Research Agent');
  });

  test('disables apply and explains the first canonical blocker', () => {
    render(
      <I18nextProvider i18n={i18n}>
        <AgentSwitchDialog
          visible
          preview={{
            ...preview,
            can_apply: false,
            handoff: { ...preview.handoff, available: false },
            blockers: [{
              code: 'AGENT_SESSION_TURN_ACTIVE',
              message: 'Wait for the active Turn before switching Agents',
            }],
          }}
          loading={false}
          applying={false}
          mode='context_only'
          onModeChange={() => {}}
          onConfirm={() => {}}
          onCancel={() => {}}
          onRecovery={() => {}}
        />
      </I18nextProvider>
    );
    expect(within(document.body).getByText(/Wait for the current response/)).toBeDefined();
    expect((within(document.body).getByRole('button', { name: 'Switch Agent' }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(document.body).getByRole('radio', { name: /Hand off the current task/ }) as HTMLInputElement).disabled).toBe(true);
  });

  test('offers the matching recovery route for a machine-readable resource blocker', () => {
    const onRecovery = mock(() => {});
    render(
      <I18nextProvider i18n={i18n}>
        <AgentSwitchDialog
          visible
          preview={{
            ...preview,
            can_apply: false,
            blockers: [{
              code: 'AGENT_SESSION_RESOURCE_REQUIRED',
              message: 'The target Agent requires another resource',
            }],
          }}
          loading={false}
          applying={false}
          mode='context_only'
          onModeChange={() => {}}
          onConfirm={() => {}}
          onCancel={() => {}}
          onRecovery={onRecovery}
        />
      </I18nextProvider>
    );
    fireEvent.click(within(document.body).getByRole('button', { name: 'Open Agent settings' }));
    expect(onRecovery).toHaveBeenCalledWith('AGENT_SESSION_RESOURCE_REQUIRED');
  });
});
