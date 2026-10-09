import { createDefaultIdmmConfig } from '../idmm';
import type {
AgentPresetDocument,
AgentPresetDraft,
CapabilityId,
CapabilityRef,
CapabilitySelection,
} from './contracts';

function canonicalizeDraftValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalizeDraftValue);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalizeDraftValue(entry)])
    );
  }
  return value;
}

const canonicalFingerprint = (value: unknown): string =>
  JSON.stringify(canonicalizeDraftValue(value));

const draftFingerprint = (draft: AgentPresetDraft): string => canonicalFingerprint(draft);

export const isDraftDirty = (
  savedDraft: AgentPresetDraft | null,
  draft: AgentPresetDraft
): boolean => savedDraft == null || draftFingerprint(savedDraft) !== draftFingerprint(draft);

export const createEmptyAgentPresetDocument = (): AgentPresetDocument => ({
  schema_version: '1.0.0',
  model_route_refs: {},
  chat_route_records: {},
  enabled_capabilities: [],

  skill_bindings: [],
  system_role_provider_overrides: {},
  persona: '',
  instructions: '',
  starter_prompts: [],
  runtime_policy: {
    idmm: createDefaultIdmmConfig(),
  },
});

/**
 * Restore fields that the wire contract may omit when they contain their
 * default value. Renderer code consumes the complete authoring model returned
 * here instead of repeating transport-shape fallbacks in individual panels.
 */
const restoreDocumentDefaults = (document: AgentPresetDocument): AgentPresetDocument => {
  document.chat_route_records ??= {};
  document.system_role_provider_overrides ??= {};
  document.starter_prompts ??= [];
  document.runtime_policy ??= { idmm: createDefaultIdmmConfig() };
  return document;
};

export const cloneAgentPresetDocument = (
  document: AgentPresetDocument
): AgentPresetDocument => restoreDocumentDefaults(structuredClone(document));

export const agentPresetDocumentsEqual = (
  left: AgentPresetDocument,
  right: AgentPresetDocument
): boolean => canonicalFingerprint(cloneAgentPresetDocument(left)) ===
  canonicalFingerprint(cloneAgentPresetDocument(right));

const selection = (capability: CapabilityRef): CapabilitySelection => ({
  capability,
  action_allowlist: [],
});

export const cloneDraft = (draft: AgentPresetDraft): AgentPresetDraft => {
  const cloned = structuredClone(draft);
  cloned.document = restoreDocumentDefaults(cloned.document);
  return cloned;
};

export type CapabilityPlacement = 'enabled' | 'none';

export function capabilityPlacement(
  document: AgentPresetDocument,
  capability: CapabilityId | CapabilityRef
): CapabilityPlacement {
  const id = typeof capability === 'string' ? capability : capability.id;
  return document.enabled_capabilities.some(item => item.capability.id === id) ? 'enabled' : 'none';
}

export function placeCapability(
  document: AgentPresetDocument,
  capability: CapabilityRef,
  placement: CapabilityPlacement
): AgentPresetDocument {
  const existing = document.enabled_capabilities.find(item => item.capability.id === capability.id);
  const enabled = document.enabled_capabilities.filter(item => item.capability.id !== capability.id);
  if (placement === 'enabled') enabled.push(existing ?? selection(capability));
  const next = { ...document, enabled_capabilities: enabled.sort((a, b) => a.capability.id.localeCompare(b.capability.id)) };
  if (placement === 'none' && document.context_order) {
    next.context_order = document.context_order.filter(id => id !== capability.id);
    if (!next.context_order.length) delete next.context_order;
  }
  if (placement === 'none' && document.middleware_order) {
    next.middleware_order = document.middleware_order.filter(id => id !== capability.id);
    if (!next.middleware_order.length) delete next.middleware_order;
  }
  return next;
}
