import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import * as mcpCatalog from '@/renderer/hooks/mcp/catalog';
import { SessionCapabilityComposerLayout } from '@/renderer/components/chat/SessionCapabilityPicker';
import CompanionCapabilityControls from './CompanionCapabilityControls';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'common.skills': 'Skills', 'common.close': 'Close',
  'nomi.chat.skillsScopeHint': 'Shared with this companion',
  'conversation.capabilityPicker.manageSkills': 'Manage skills',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test.each([true, false])('MCP uses its own shared-session update and honors Agent capability (enabled=%s)', async (enabled) => {
  const id = '019b0000-0000-7000-8000-000000000003';
  const conversation = { id: '019b0000-0000-7000-8000-000000000002', extra: { mcp_server_ids: [] }, agent_snapshot: { enabled_capabilities: enabled ? [`nomi.mcp.v1.${'a'.repeat(64)}`] : [] } } as any;
  const available = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  const auto = spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  const mcp = spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [{ mcp_server_id: id, name: 'Shared MCP', enabled: true, builtin: false, tools: [] }], enabledServers: [] } as any);
  const save = spyOn(ipcBridge.agentPlatform.sessions.updateMcpSelection, 'invoke').mockResolvedValue({});
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  for (const spy of [available, auto, mcp, save, get]) restores.push(() => spy.mockRestore());
  const patchCompanion = mock(async () => undefined);
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <SessionCapabilityComposerLayout picker={<CompanionCapabilityControls conversation={conversation} companion={{
      profile: { companion_id: 'companion-rail', skills: { enabled: [], disabled_auto: [] } }, patchCompanion,
    } as any} />}><textarea /></SessionCapabilityComposerLayout>
  </I18nextProvider></MemoryRouter>);
  fireEvent.click(await view.findByRole('button', { name: 'MCP · 0' }));
  const checkbox = await view.findByRole('checkbox', { name: 'Shared MCP' });
  expect((checkbox as HTMLInputElement).disabled).toBe(!enabled);
  fireEvent.click(checkbox);
  if (enabled) await waitFor(() => expect(save).toHaveBeenCalledWith({ agent_session_id: conversation.id, mcp_server_ids: [id] }));
  else expect(save).not.toHaveBeenCalled();
  expect(patchCompanion).not.toHaveBeenCalled();
});

test('uses the shared popup and saves companion skill intent without session overrides', async () => {
  const skills = ['auto-skill', 'optional-skill'].map((name) => ({ name, description: name, location: '', source: 'builtin' as const, is_custom: false }));
  const available = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue(skills);
  const auto = spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([skills[0]]);
  const mcp = spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [], enabledServers: [] } as any);
  const session = spyOn(ipcBridge.agentPlatform.sessions.updateCapabilitySelection, 'invoke');
  for (const spy of [available, auto, mcp, session]) restores.push(() => spy.mockRestore());
  const patches: unknown[] = [];
  function View() {
    const [profile, setProfile] = useState({ companion_id: 'companion-rail', skills: { enabled: ['uninstalled-skill'], disabled_auto: [] as string[] } });
    const location = useLocation();
    return <><output>{location.pathname}{location.search}</output>
      <SessionCapabilityComposerLayout picker={<CompanionCapabilityControls conversation={{ id: '019b0000-0000-7000-8000-000000000002', extra: {}, agent_snapshot: { enabled_capabilities: [`nomi.mcp.v1.${'a'.repeat(64)}`] } } as any} companion={{ profile, patchCompanion: async (patch: any) => {
        patches.push(patch); setProfile((previous) => ({ ...previous, ...patch }));
      } } as any} />}><textarea aria-label='Message' /></SessionCapabilityComposerLayout>
    </>;
  }
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}><View /></I18nextProvider></MemoryRouter>);
  const trigger = await view.findByRole('button', { name: 'Skills · 1' });
  fireEvent.click(trigger);
  expect(await view.findByRole('dialog', { name: 'Skills' })).toBeTruthy();
  expect(view.getByText('Shared with this companion')).toBeTruthy();
  expect(view.getByTestId('session-mcp-trigger')).toBeTruthy();
  fireEvent.click(view.getByRole('checkbox', { name: 'auto-skill' }));
  await waitFor(() => expect(patches).toEqual([{ skills: { enabled: ['uninstalled-skill'], disabled_auto: ['auto-skill'] } }]));
  expect(session).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
  fireEvent.click(view.getByRole('button', { name: 'Skills · 0' }));
  fireEvent.click(await view.findByRole('button', { name: 'Manage skills' }));
  await waitFor(() => expect(view.getByRole('status').textContent).toBe('/nomi?companion=companion-rail&tab=skills'));
});
