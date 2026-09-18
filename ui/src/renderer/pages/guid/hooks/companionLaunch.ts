import { ipcBridge } from '@/common';
import type { TProviderWithModel } from '@/common/config/storage';
import type { AgentResourceSelection } from '@/common/types/agentPlatform';
import { conversationTarget, parseChannelPluginId, parseCompanionId, type ConversationId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { persistCompanionTurnDelivery, completeCompanionTurnDelivery, readCompanionTurnDelivery } from '@/renderer/pages/companion/companionTurnDelivery';

/** All launch surfaces use the product's canonical Conversation and settings. */
export async function prepareCompanionConversation(
  resources: AgentResourceSelection[],
  fallbackModel?: TProviderWithModel,
) {
  const selected = resources.find((item) => item.resource_kind === 'companion');
  if (!selected) throw new Error('RESOURCE_SELECTION_REQUIRED');
  const companion_id = parseCompanionId(selected.resource_id);
  const profile = await ipcBridge.companion.getCompanion.invoke({ companion_id });
  let model = profile.model;
  if (!profile.model?.provider_id || !profile.model.model) {
    if (!fallbackModel) throw new Error('MODEL_REQUIRED');
    model = { provider_id: fallbackModel.id, model: fallbackModel.use_model };
    await ipcBridge.companion.patchCompanion.invoke({ companion_id, patch: {
      model,
    } });
  }
  if (!model?.provider_id || !model.model) throw new Error('MODEL_REQUIRED');
  const channelId = resources.find((item) => item.resource_kind === 'channel')?.resource_id;
  const robotId = resources.find((item) => item.resource_kind === 'robot')?.resource_id;
  let bindChannel = false;
  let bindRobot = false;
  // Recheck the live owner before applying a staged choice. Never silently
  // reassign another companion's device or a customer-service channel.
  if (channelId) {
    const channel = (await ipcBridge.channel.getPluginStatus.invoke()).find((item) => item.plugin_id === channelId);
    if (!channel || channel.owner_domain !== 'companion' || (channel.companionId && channel.companionId !== companion_id)) {
      throw new Error('RESOURCE_OWNER_MISMATCH');
    }
    bindChannel = channel.companionId !== companion_id;
  }
  if (robotId) {
    const robot = (await ipcBridge.robot.list.invoke()).find((item) => item.robot_id === robotId);
    if (!robot || (robot.companion_id && robot.companion_id !== companion_id)) throw new Error('RESOURCE_OWNER_MISMATCH');
    bindRobot = robot.companion_id !== companion_id;
  }
  if (bindChannel && channelId) await ipcBridge.channel.setChannelCompanion.invoke({ plugin_id: parseChannelPluginId(channelId), companion_id });
  if (bindRobot && robotId) await ipcBridge.robot.update.invoke({ robot_id: robotId, updates: { companion_id } });
  const active = await ipcBridge.companion.getCompanionSession.invoke({ companion_id });
  await ipcBridge.agentPlatform.selectProductBinding.invoke({
    target_kind: 'companion',
    target_id: companion_id,
    request: {
      selection: { kind: 'template', template_key: 'companion.default' },
      model,
      resource_selections: resources,
      ...(active.conversation_id ? { conversation_id: active.conversation_id } : {}),
    },
  });
  const thread = await ipcBridge.companion.ensureCompanionSession.invoke({ companion_id });
  const conversation = await ipcBridge.conversation.get.invoke({ conversation_id: thread.conversation_id });
  if (!conversation?.id) throw new Error('Companion Conversation is unavailable');
  return conversation;
}

export async function sendCompanionLaunchMessage(conversationId: ConversationId, input: string, files: string[]) {
  const key = sessionStorageKey('guid-companion-delivery', conversationTarget(conversationId));
  const pending = readCompanionTurnDelivery(sessionStorage, key);
  if (pending && (pending.input !== input || JSON.stringify(pending.files) !== JSON.stringify(files))) {
    throw new Error('COMPANION_PREVIOUS_DELIVERY_PENDING');
  }
  const delivery = persistCompanionTurnDelivery(sessionStorage, key, conversationId, input, files);
  const result = await ipcBridge.conversation.sendMessage.invoke(delivery);
  if (!result) throw new Error('Companion message was not accepted');
  completeCompanionTurnDelivery(sessionStorage, key, conversationId, delivery.idempotency_key);
}
