import type { PluginDeliveryRequirement } from '@/common/types/pluginDevelopment';
import type { PluginLaunchIntent } from './pluginConversationLaunch';

/** Reading an existing implementation is a normal turn; edits start a delivery task. */
export function initialPluginDelivery(
  intent: PluginLaunchIntent | null,
  input: string,
  bootstrapInput: string,
): PluginDeliveryRequirement | undefined {
  if (intent && !intent.requirement && (intent.plugin_id || intent.draft_id)
      && input.trim() === bootstrapInput.trim()) return undefined;
  return {};
}
