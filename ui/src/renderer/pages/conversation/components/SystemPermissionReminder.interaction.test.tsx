import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { asAgentPresetId, type AgentResolvedSnapshot } from '@/common/types/agentPlatform';
import { parseConversationId } from '@/common/types/ids';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import SystemPermissionReminder from './SystemPermissionReminder';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { conversation, settings } } },
  interpolation: { escapeValue: false },
});

const status: SystemPermissionStatus = {
  platform: 'macos',
  app_label: 'NomiFun',
  permissions: [
    { kind: 'microphone', state: 'granted', can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['voice_input'] },
    { kind: 'accessibility', state: 'not_determined', can_request: true, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['computer_use'] },
    { kind: 'screen_recording', state: 'granted', can_request: false, can_open_settings: true, requires_restart_after_grant: true, capabilities: ['computer_use'] },
  ],
};

const snapshot = (actions: string[]): AgentResolvedSnapshot => ({
  preset_id: asAgentPresetId('01900000-0000-7000-8000-000000000001'),
  preset_revision: 1,
  preset_name: 'Computer Agent',
  instructions: '',
  included_skills: [],
  excluded_auto_skills: [],
  enabled_capabilities: ['computer'],
  enabled_capability_actions: { computer: actions },
  required_resource_kinds: ['computer'],
  knowledge_policy: { enabled: false, writeback: false, grounded: false },
  warnings: [],
});

afterEach(() => cleanup());

test('auto-opens a recoverable bubble for missing frozen Computer permissions', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status);
  const Location = () => <span data-testid='location'>{useLocation().pathname}{useLocation().search}</span>;
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/conversation/session-1']}>
        <SystemPermissionReminder
          conversationId={parseConversationId('01900000-0000-7000-8000-000000000002')}
          snapshot={snapshot(['computer/input'])}
        />
        <Location />
      </MemoryRouter>
    </I18nextProvider>
  );

  expect(await screen.findByText(conversation.systemPermissions.title)).toBeTruthy();
  expect(screen.getByText(/Accessibility/)).toBeTruthy();
  const trigger = screen.getByTestId('system-permission-reminder');
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(screen.getByRole('button', { name: conversation.systemPermissions.later }));
  await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
  await act(async () => { window.dispatchEvent(new Event('focus')); });
  await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(trigger);
  await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('true'));
  fireEvent.click(screen.getByRole('button', { name: conversation.systemPermissions.openSettings }));
  await waitFor(() => expect(screen.getByTestId('location').textContent).toBe('/settings/permissions?tab=computer-use'));
  get.mockRestore();
});

test('launch-only Computer capability neither probes nor renders a reminder', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke');
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <SystemPermissionReminder
          conversationId={parseConversationId('01900000-0000-7000-8000-000000000003')}
          snapshot={snapshot(['computer/launch'])}
        />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(screen.queryByTestId('system-permission-reminder')).toBeNull();
  expect(get).not.toHaveBeenCalled();
  get.mockRestore();
});
