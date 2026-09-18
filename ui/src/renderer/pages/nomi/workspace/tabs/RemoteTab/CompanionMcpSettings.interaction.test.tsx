import '../../../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { parseCompanionId, parseConversationId, parseMcpServerId } from '@/common/types/ids';
import CompanionMcpSettings from './CompanionMcpSettings';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'agentSettings.resources.kinds.mcpConnection': 'MCP connection',
  'nomi.chat.mcpScopeHint': 'Shared with this companion',
} } } });
const restores: (() => void)[] = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test.each([false, true])('management reads the shared MCP selection and retains it on save failure (%s)', async (fail) => {
  const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
  const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000002');
  const serverId = parseMcpServerId('019b0000-0000-7000-8000-000000000003');
  track(spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockResolvedValue({ conversation_id: conversationId }));
  track(spyOn(ipcBridge.companion.ensureCompanionSession, 'invoke').mockResolvedValue({ conversation_id: conversationId } as any));
  track(spyOn(ipcBridge.mcpService.listServers, 'invoke').mockResolvedValue([{ mcp_server_id: serverId, name: 'Shared MCP', enabled: true }] as any));
  track(spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, type: 'nomi', extra: { mcp_server_ids: [serverId] }, agent_snapshot: { enabled_capabilities: [`nomi.mcp.v1.${'a'.repeat(64)}`] } } as any));
  const save = track(spyOn(ipcBridge.agentPlatform.sessions.updateMcpSelection, 'invoke'));
  if (fail) save.mockRejectedValue(new Error('busy'));
  else save.mockResolvedValue({});
  track(spyOn(Message, 'error').mockReturnValue(() => {}));
  const view = render(<I18nextProvider i18n={i18n}><CompanionMcpSettings companionId={companionId} /></I18nextProvider>);
  const select = view.getByRole('combobox', { name: 'MCP connection' });
  await waitFor(() => expect(select.textContent).toContain('Shared MCP'));
  fireEvent.click(select);
  const options = await view.findAllByText('Shared MCP');
  fireEvent.click(options[options.length - 1]);
  await waitFor(() => expect(save).toHaveBeenCalledWith({ agent_session_id: conversationId, mcp_server_ids: [] }));
  await waitFor(() => expect(select.textContent?.includes('Shared MCP')).toBe(fail));
});
