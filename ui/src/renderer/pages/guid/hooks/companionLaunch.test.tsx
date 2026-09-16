import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { parseCompanionId, parseConversationId, parseProviderId, parseChannelPluginId, parseMcpServerId } from '@/common/types/ids';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { prepareCompanionConversation, sendCompanionLaunchMessage } from './companionLaunch';
import { useGuidSend, type GuidSendDeps } from './useGuidSend';

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000002');
const providerId = parseProviderId('019b0000-0000-7000-8000-000000000003');
const channelId = parseChannelPluginId('019b0000-0000-7000-8000-000000000004');
const mcpId = parseMcpServerId('019b0000-0000-7000-8000-000000000005');
const restores: (() => void)[] = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); sessionStorage.clear(); });

function setup() {
  setBrowserStorageGeneration('019b0000-0000-7000-8000-000000000099');
  const profile = { companion_id: companionId, model: { provider_id: providerId, model: 'companion-model' } };
  const conversation = { id: conversationId, type: 'nomi', extra: { companion_session: true, companion_id: companionId, mcp_server_ids: [mcpId] } };
  track(spyOn(ipcBridge.companion.getCompanion, 'invoke').mockResolvedValue(profile as any));
  const ensure = track(spyOn(ipcBridge.companion.ensureCompanionSession, 'invoke').mockResolvedValue({ conversation_id: conversationId } as any));
  const patch = track(spyOn(ipcBridge.companion.patchCompanion, 'invoke').mockResolvedValue(profile as any));
  track(spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation as any));
  const create = track(spyOn(ipcBridge.agentPlatform.sessions.create, 'invoke').mockRejectedValue(new Error('must not create another session')));
  const mcp = track(spyOn(ipcBridge.agentPlatform.sessions.updateMcpSelection, 'invoke').mockResolvedValue({}));
  const send = track(spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockResolvedValue({ msg_id: 'accepted' } as any));
  return { ensure, patch, create, mcp, send };
}

test('home companion launch reuses the sidebar conversation and sends into existing history', async () => {
  const api = setup();
  const navigate = mock(async (_target: string) => undefined);
  const noop = () => undefined;
  const deps = {
    input: 'Continue our conversation', files: ['example.txt'], dir: 'unused-workspace', loading: false,
    selection: { kind: 'template', templateKey: 'companion.default' }, selectedTemplate: { template_key: 'companion.default' },
    resourceSelections: [{ resource_kind: 'companion', resource_id: companionId }], resourceResolutionReady: true,
    current_model: undefined, autoWork: { enabled: false }, workspaceEnabled: false,
    setInput: noop, setFiles: noop, setDir: noop, setLoading: noop, setMentionOpen: noop, setMentionQuery: noop,
    setMentionSelectorOpen: noop, setMentionActiveIndex: noop, navigate, t: (key: string) => key,
  } as unknown as GuidSendDeps;
  const hook = renderHook(() => useGuidSend(deps));
  expect(hook.result.current.isButtonDisabled).toBe(false);
  await act(async () => { await hook.result.current.handleSend('browser'); });
  expect(api.send).not.toHaveBeenCalled();
  await act(async () => { await hook.result.current.handleSend(); });
  expect(api.ensure).toHaveBeenCalledTimes(2);
  expect(api.create).not.toHaveBeenCalled();
  expect(api.patch).not.toHaveBeenCalled();
  expect(api.mcp).not.toHaveBeenCalled();
  expect(navigate.mock.calls.map((call) => call[0])).toEqual([`/conversation/${conversationId}`, `/conversation/${conversationId}`]);
  expect(api.send.mock.calls[0][0]).toMatchObject({ conversation_id: conversationId, input: deps.input, files: deps.files });
  expect(api.send.mock.calls[0][0]).not.toHaveProperty('initial_only');
  expect(api.send.mock.calls[0][0]).not.toHaveProperty('preset_id');
});

test('selected channel, robot and MCP persist through the same APIs as companion management', async () => {
  const api = setup();
  track(spyOn(ipcBridge.channel.getPluginStatus, 'invoke').mockResolvedValue([{ plugin_id: channelId, owner_domain: 'companion' }] as any));
  track(spyOn(ipcBridge.robot.list, 'invoke').mockResolvedValue([{ robot_id: 'device', companion_id: null }] as any));
  const channel = track(spyOn(ipcBridge.channel.setChannelCompanion, 'invoke').mockResolvedValue(undefined));
  const robot = track(spyOn(ipcBridge.robot.update, 'invoke').mockResolvedValue({} as any));
  await prepareCompanionConversation([
    { resource_kind: 'companion', resource_id: companionId }, { resource_kind: 'channel', resource_id: channelId },
    { resource_kind: 'robot', resource_id: 'device' }, { resource_kind: 'mcp_server', resource_id: mcpId },
  ]);
  expect(channel).toHaveBeenCalledWith({ plugin_id: channelId, companion_id: companionId });
  expect(robot).toHaveBeenCalledWith({ robot_id: 'device', updates: { companion_id: companionId } });
  expect(api.mcp).toHaveBeenCalledWith({ agent_session_id: conversationId, mcp_server_ids: [mcpId] });
});

test('stale resource ownership rejects launch and never sends or rebinds another companion', async () => {
  const api = setup();
  track(spyOn(ipcBridge.channel.getPluginStatus, 'invoke').mockResolvedValue([{ plugin_id: channelId, owner_domain: 'customer_service' }] as any));
  const channel = track(spyOn(ipcBridge.channel.setChannelCompanion, 'invoke').mockResolvedValue(undefined));
  await expect(prepareCompanionConversation([{ resource_kind: 'companion', resource_id: companionId }, { resource_kind: 'channel', resource_id: channelId }])).rejects.toThrow('RESOURCE_OWNER_MISMATCH');
  expect(channel).not.toHaveBeenCalled();
  expect(api.send).not.toHaveBeenCalled();
});

test('a lost message response retries the same delivery key instead of duplicating the turn', async () => {
  const api = setup();
  api.send.mockRejectedValueOnce(new Error('response lost'));
  await expect(sendCompanionLaunchMessage(conversationId, 'hello', [])).rejects.toThrow('response lost');
  await sendCompanionLaunchMessage(conversationId, 'hello', []);
  expect(api.send.mock.calls[0][0].idempotency_key).toBe(api.send.mock.calls[1][0].idempotency_key);
});
