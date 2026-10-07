/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import {
  capabilityInputFromResponse,
  capabilityInputsFromDefinition,
  type ModelDefinitionDraft,
  type ProviderModelCapabilityInput,
} from './providerModelAdvanced';

type TaskCapabilitySave =
  | { capabilities: ProviderModelCapabilityInput[]; error?: never }
  | { capabilities?: never; error: 'unavailable' | 'changed' | 'invalid' };

const normalizedText = (value: string | null | undefined): string | null => value?.trim() || null;

/** Shared model text can update independently while a task form is open. */
export const resolveScopedModelTextEdit = (
  draft: string | null | undefined,
  baseline: string | null | undefined,
  latest: string | null | undefined
): { value: string | null; conflict: false } | { conflict: true; value?: never } => {
  const draftValue = normalizedText(draft);
  const baselineValue = normalizedText(baseline);
  const latestValue = normalizedText(latest);
  if (draftValue === baselineValue) return { value: latestValue, conflict: false };
  if (latestValue !== baselineValue && latestValue !== draftValue) return { conflict: true };
  return { value: draftValue, conflict: false };
};

/**
 * The backend saves the whole capability list. Editing one scenario replaces
 * exactly that task and carries every other task from the latest persisted
 * model, rather than replaying the snapshot taken when the editor was opened.
 */
export const mergeTaskCapabilityEdit = (
  definition: ModelDefinitionDraft,
  latestCapabilities: readonly ProviderModelCapabilityResponse[],
  task: ModelTask,
  baseline: ProviderModelCapabilityInput | undefined
): TaskCapabilitySave => {
  const current = latestCapabilities.filter((capability) => capability.task === task);
  const edited = definition.capabilities.filter((capability) => capability.task === task);
  if (current.length !== 1 || edited.length !== 1 || baseline?.task !== task) {
    return { error: 'unavailable' };
  }
  if (JSON.stringify(capabilityInputFromResponse(current[0])) !== JSON.stringify(baseline)) {
    return { error: 'changed' };
  }
  const inputs = capabilityInputsFromDefinition({ ...definition, capabilities: edited });
  if (!inputs) return { error: 'invalid' };
  return {
    capabilities: latestCapabilities.map((capability) =>
      capability.task === task ? inputs[0] : capabilityInputFromResponse(capability)
    ),
  };
};
