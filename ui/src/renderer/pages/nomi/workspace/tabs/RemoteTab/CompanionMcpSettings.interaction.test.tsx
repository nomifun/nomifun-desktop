import '../../../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionProfile, ICompanionProfilePatch } from '@/common/adapter/ipcBridge';
import { parseCompanionId, parseMcpServerId } from '@/common/types/ids';
import CompanionMcpSettings from './CompanionMcpSettings';
import { notifyMcpCatalogChanged } from '@/renderer/hooks/mcp/catalog';
import type { CompanionHandle } from '../../types';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'agentSettings.resources.kinds.mcpConnection': 'MCP connection',
  'nomi.cohabit.extensionDefaultsHint': 'Shared future defaults. Existing Sessions keep their settings.',
  'agentSettings.productBinding.saveFailed': 'Save failed',
} } } });
const restores: (() => void)[] = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const enabledId = parseMcpServerId('019b0000-0000-7000-8000-000000000003');
const disabledId = parseMcpServerId('019b0000-0000-7000-8000-000000000004');
const enabled = { mcp_server_id: enabledId, name: 'Available MCP', enabled: true };
const stopped = { mcp_server_id: disabledId, name: 'Stopped MCP', enabled: false };
const profile = (ids: string[]) => ({ companion_id: companionId, mcp_servers: ids } as ICompanionProfile);
const mount = (ids: string[], patchCompanion: CompanionHandle['patchCompanion']) => render(<I18nextProvider i18n={i18n}>
  <CompanionMcpSettings companion={{ profile: profile(ids), patchCompanion } as CompanionHandle} />
</I18nextProvider>);
const install = () => {
  const listing = track(spyOn(ipcBridge.mcpService.listServers, 'invoke').mockResolvedValue([enabled, stopped] as any));
  const getSession = track(spyOn(ipcBridge.companion.getCompanionSession, 'invoke'));
  const putSelection = track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke'));
  track(spyOn(Message, 'success').mockImplementation(() => () => {}));
  return { listing, getSession, putSelection };
};

test('MCP management saves one shared default without changing an existing input Session', async () => {
  const { getSession, putSelection } = install();
  const patches: ICompanionProfilePatch[] = [];
  const view = mount([], async (patch, options) => { patches.push(patch); expect(options).toEqual({ optimistic: false }); return profile(patch.mcp_servers!); });
  await waitFor(() => expect(view.getByRole('combobox', { name: 'MCP connection' }).getAttribute('aria-disabled')).not.toBe('true'));
  fireEvent.click(view.getByRole('combobox', { name: 'MCP connection' }));
  fireEvent.click(await view.findByText(enabled.name));
  await waitFor(() => expect(patches).toEqual([{ mcp_servers: [enabledId] }]));
  expect(view.getByText('Shared future defaults. Existing Sessions keep their settings.')).toBeTruthy();
  expect(getSession).not.toHaveBeenCalled();
  expect(putSelection).not.toHaveBeenCalled();
});

test('catalog refresh preserves disabled stored defaults and never silently rewrites permissions', async () => {
  const { listing, putSelection } = install();
  let writes = 0;
  const view = mount([enabledId, disabledId], async () => { writes++; return profile([]); });
  const select = view.getByRole('combobox', { name: 'MCP connection' });
  await waitFor(() => expect(select.textContent).toContain(enabled.name));
  expect(select.textContent).toContain(disabledId);
  fireEvent.click(select);
  expect(view.queryByText(stopped.name)).toBeNull();
  listing.mockResolvedValue([{ ...enabled, enabled: false }, stopped] as any);
  await act(async () => notifyMcpCatalogChanged());
  await waitFor(() => expect(select.textContent).toContain(enabledId));
  expect(writes).toBe(0);
  expect(putSelection).not.toHaveBeenCalled();
});

test('a failed shared default write retains the committed selection', async () => {
  install();
  const view = mount([], async () => { throw new Error('private configuration path'); });
  const select = view.getByRole('combobox', { name: 'MCP connection' });
  await waitFor(() => expect(select.getAttribute('aria-disabled')).not.toBe('true'));
  fireEvent.click(select);
  fireEvent.click(await view.findByText(enabled.name));
  await waitFor(() => expect(view.getByRole('alert').textContent).toBe('Save failed'));
  expect(select.textContent).not.toContain(enabled.name);
  expect(view.baseElement.textContent).not.toContain('private configuration path');
});
