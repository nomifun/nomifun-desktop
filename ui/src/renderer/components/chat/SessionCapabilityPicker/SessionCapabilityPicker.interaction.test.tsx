import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import SessionCapabilityPicker, { SessionCapabilityComposerLayout, type SessionCapabilityDraft } from './index';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'common.skills': 'Skills', 'common.close': 'Close',
  'conversation.capabilityPicker.nextSendApplyNote': 'This session · applies on next send',
  'conversation.capabilityPicker.unavailable': 'Removed or unavailable',
  'conversation.capabilityPicker.error': 'Unavailable',
} } } });
afterEach(() => cleanup());
const skill = { name: 'my-skill', description: 'My instructions', location: '', source: 'custom' as const, is_custom: true, auto: false };
const server = { mcp_server_id: '019b0000-0000-7000-8000-000000000003', name: 'Global MCP', enabled: true, last_test_status: 'connected', tools: [{ name: 'query' }] } as any;
const catalog = { skills: [skill], autoSkillNames: new Set<string>(), mcpServers: [server] };
function View({ disabled = false, missing = false, unavailable = false, selected = false, unreadyMcp = false, selectedMcp = false }: { disabled?: boolean; missing?: boolean; unavailable?: boolean; selected?: boolean; unreadyMcp?: boolean; selectedMcp?: boolean }) {
  const [draft, setDraft] = useState<SessionCapabilityDraft>({ skillNames: missing ? ['removed-skill'] : selected ? ['my-skill'] : [], mcpServerIds: selectedMcp ? [server.mcp_server_id] : [] });
  const currentCatalog = unavailable ? { ...catalog, skills: [{ ...skill, session_available: false, session_error: 'Resource is a symlink outside this package' }] } : unreadyMcp ? { ...catalog, mcpServers: [{ ...server, tools: [] }] } : catalog;
  return <MemoryRouter><I18nextProvider i18n={i18n}><SessionCapabilityComposerLayout picker={<SessionCapabilityPicker catalog={currentCatalog}
    draft={draft} onChange={setDraft} applyMode='next-send' disabled={disabled} />}><textarea aria-label='Message' /></SessionCapabilityComposerLayout>
    <output>{JSON.stringify(draft)}</output>
  </I18nextProvider></MemoryRouter>;
}

test('composer exposes separate Skill and MCP icons and edits the next-send draft', async () => {
  const view = render(<View />);
  fireEvent.click(view.getByRole('button', { name: 'Skills · 0' }));
  fireEvent.click(await view.findByRole('checkbox', { name: 'my-skill' }));
  expect(view.getByRole('status').textContent).toContain('my-skill');
  fireEvent.click(view.getByRole('button', { name: 'MCP · 0' }));
  fireEvent.click(await view.findByRole('checkbox', { name: 'Global MCP' }));
  expect(view.getByRole('status').textContent).toContain(server.mcp_server_id);
  expect(view.getByRole('button', { name: 'MCP · 1' })).toBeTruthy();
  expect(view.getByText('This session · applies on next send')).toBeTruthy();
});

test('active and read-only sessions allow inspecting disabled choices without mutation', async () => {
  const view = render(<View disabled />);
  for (const [trigger, label] of [['Skills · 0', 'my-skill'], ['MCP · 0', 'Global MCP']]) {
    fireEvent.click(view.getByRole('button', { name: trigger }));
    const checkbox = await view.findByRole('checkbox', { name: label });
    expect((checkbox as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(checkbox);
  }
  expect(view.getByRole('status').textContent).toBe('{"skillNames":[],"mcpServerIds":[]}');
});

test('removed selected Skills remain visible and can be explicitly deselected', async () => {
  const view = render(<View missing />);
  fireEvent.click(view.getByRole('button', { name: 'Skills · 1' }));
  const checkbox = await view.findByRole('checkbox', { name: 'removed-skill' });
  expect(view.getByText('Removed or unavailable')).toBeTruthy();
  fireEvent.click(checkbox);
  await waitFor(() => expect(view.queryByRole('checkbox', { name: 'removed-skill' })).toBeNull());
  expect(view.getByRole('button', { name: 'Skills · 0' })).toBeTruthy();
});


test.each([false, true])('an unavailable Skill shows its reason and can only be deselected (selected=%s)', async (selected) => {
  const view = render(<View unavailable selected={selected} />);
  fireEvent.click(view.getByRole('button', { name: `Skills · ${selected ? 1 : 0}` }));
  const checkbox = await view.findByRole('checkbox', { name: 'my-skill' });
  expect((checkbox as HTMLInputElement).disabled).toBe(!selected);
  expect(view.getByText('My instructions')).toBeTruthy();
  expect(view.getByTitle('Resource is a symlink outside this package')).toBeTruthy();
  expect(view.getByText('Unavailable')).toBeTruthy();
  fireEvent.click(checkbox);
  expect(view.getByRole('status').textContent).toBe('{"skillNames":[],"mcpServerIds":[]}');
  expect((view.getByRole('checkbox', { name: 'my-skill' }) as HTMLInputElement).disabled).toBe(true);
  fireEvent.click(view.getByText('my-skill', { selector: 'strong' }));
  expect(view.getByRole('status').textContent).toBe('{"skillNames":[],"mcpServerIds":[]}');
});


test.each([false, true])('an MCP without tested tools cannot be newly selected but can be removed (selected=%s)', async (selectedMcp) => {
  const view = render(<View unreadyMcp selectedMcp={selectedMcp} />);
  fireEvent.click(view.getByRole('button', { name: `MCP · ${selectedMcp ? 1 : 0}` }));
  const checkbox = await view.findByRole('checkbox', { name: 'Global MCP' });
  expect((checkbox as HTMLInputElement).disabled).toBe(!selectedMcp);
  fireEvent.click(checkbox);
  expect(view.getByRole('status').textContent).toBe('{"skillNames":[],"mcpServerIds":[]}');
});
