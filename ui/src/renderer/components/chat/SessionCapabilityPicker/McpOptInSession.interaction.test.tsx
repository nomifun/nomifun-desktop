import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { AgentSessionCapabilitySelectionState } from '@/common/types/agentPlatform';
import { parseEntityId, parseMcpServerId } from '@/common/types/ids';
import SessionCapabilityPicker from './index';
import type { SessionCapabilityCatalog } from './model';
import { useSessionCapabilitySelection } from './useSessionCapabilitySelection';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: { 'common.skills': 'Skills' } } } });
const sessionId = parseEntityId('agent-session', '019b0000-0000-7000-8000-000000000002');
const disabledServerId = parseMcpServerId('019b0000-0000-7000-8000-000000000003');
const enabledServerId = parseMcpServerId('019b0000-0000-7000-8000-000000000004');
const canonical: AgentSessionCapabilitySelectionState = {
  selection: { skill_names: ['retained-skill'], mcp_server_ids: [disabledServerId] },
  binding_version: 4,
  editable: true,
};
const catalog: SessionCapabilityCatalog = {
  skills: [{ name: 'retained-skill', description: 'Retained instructions', location: '', source: 'custom', is_custom: true, auto: false }],
  autoSkillNames: new Set(),
  mcpServers: [
    { mcp_server_id: disabledServerId, name: 'Disabled MCP', enabled: false, last_test_status: 'connected', tools: [{ name: 'query' }], transport: { type: 'stdio', command: 'disabled-mcp' }, original_json: '{}', created_at: 1, updated_at: 1 },
    { mcp_server_id: enabledServerId, name: 'Enabled MCP', enabled: true, last_test_status: 'connected', tools: [{ name: 'query' }], transport: { type: 'stdio', command: 'enabled-mcp' }, original_json: '{}', created_at: 1, updated_at: 1 },
  ],
};
const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T): T => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

function Session() {
  const selection = useSessionCapabilitySelection(sessionId);
  return <>
    <SessionCapabilityPicker catalog={catalog} draft={selection.draft} onChange={selection.setDraft}
      loading={selection.loading} disabled={selection.state?.editable !== true} applyMode='next-send' />
    <output data-testid='draft'>{JSON.stringify(selection.draft)}</output>
    <output data-testid='canonical'>{JSON.stringify(selection.state)}</output>
    <button disabled={selection.loading} onClick={() => { void selection.applyBeforeSend(); }}>Apply for next send</button>
  </>;
}

test('hiding a disabled MCP changes only the local draft until explicit canonical admission', async () => {
  let changed!: Parameters<typeof ipcBridge.agentPlatform.sessions.onCapabilitiesChanged.on>[0];
  track(spyOn(ipcBridge.agentPlatform.sessions.onCapabilitiesChanged, 'on').mockImplementation((handler) => { changed = handler; return () => {}; }));
  track(spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {}));
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(canonical));
  const admitted = { ...canonical, binding_version: 5, selection: { skill_names: ['retained-skill'], mcp_server_ids: [] } };
  const put = track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockResolvedValue(admitted));
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}><Session /></I18nextProvider></MemoryRouter>);
  const draft = () => JSON.parse(view.getByTestId('draft').textContent!);
  const state = () => JSON.parse(view.getByTestId('canonical').textContent!);
  await waitFor(() => expect(draft()).toEqual({ skillNames: ['retained-skill'], mcpServerIds: [] }));
  await act(async () => fireEvent.click(view.getByRole('button', { name: 'MCP · 0' })));
  expect(view.queryByRole('checkbox', { name: 'Disabled MCP' })).toBeNull();
  expect(view.queryByRole('checkbox', { name: disabledServerId })).toBeNull();
  expect((view.getByRole('checkbox', { name: 'Enabled MCP' }) as HTMLInputElement).checked).toBe(false);
  expect(state()).toEqual(canonical);
  expect(put).not.toHaveBeenCalled();

  // A late unchanged canonical fact cannot restore a draft that the picker
  // already removed after observing the authoritative disabled state.
  act(() => changed({ agent_session_id: sessionId, ...canonical }));
  expect(draft()).toEqual({ skillNames: ['retained-skill'], mcpServerIds: [] });
  expect(state()).toEqual(canonical);
  expect(put).not.toHaveBeenCalled();

  await act(async () => fireEvent.click(view.getByRole('button', { name: 'Apply for next send' })));
  await waitFor(() => expect(state()).toEqual(admitted));
  expect(put).toHaveBeenCalledTimes(1);
  expect(put.mock.calls[0]?.[0]).toEqual({ agent_session_id: sessionId, expected_binding_version: 4, selection: admitted.selection });
  expect(get).toHaveBeenCalledTimes(1);
});
