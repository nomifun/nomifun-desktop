import type { AgentUiContribution } from '../types/pluginRuntimePlatform';

/** Exact presentation consent identity; labels and publication epoch are not identity. */
export const agentUiChoiceKey = (choice: AgentUiContribution) => JSON.stringify([
  choice.plugin_id, choice.capability.id, choice.capability.version, choice.expected_release_digest,
]);
