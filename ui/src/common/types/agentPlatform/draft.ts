import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  AgentPresetDraft,
  CapabilityId,
  CapabilitySelection,
  ExactCatalogRef,
  OfficialPresetTemplate,
  SkillCatalogItem,
} from './contracts';

export function canonicalizeDraftValue(value: unknown): unknown {
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

export const draftFingerprint = (draft: AgentPresetDraft): string =>
  JSON.stringify(canonicalizeDraftValue(draft));

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
});

const selection = (capability: ExactCatalogRef<'capability'>): CapabilitySelection => ({
  capability,
  action_allowlist: [],
});

export const draftFromOfficialTemplate = (
  presetId: AgentPresetDraft['preset_id'],
  template: OfficialPresetTemplate,
  displayName: string
): AgentPresetDraft => ({
  preset_id: presetId,
  display_name: displayName,
  source_template_key: template.template_key,
  document: {
    ...createEmptyAgentPresetDocument(),
    enabled_capabilities: template.seed.enabled_capabilities.map(selection),

    skill_bindings: template.seed.skill_bindings,
  },
});

export const cloneDraft = (draft: AgentPresetDraft): AgentPresetDraft => {
  const cloned = structuredClone(draft);
  cloned.document.chat_route_records ??= {};
  cloned.document.starter_prompts ??= [];
  return cloned;
};

export type CapabilityPlacement = 'enabled' | 'none';

export function capabilityPlacement(
  document: AgentPresetDocument,
  capability: CapabilityId | ExactCatalogRef<'capability'>
): CapabilityPlacement {
  const id = typeof capability === 'string' ? capability : capability.id;
  const version = typeof capability === 'string' ? undefined : capability.version;
  return document.enabled_capabilities.some(item => item.capability.id === id &&
    (version === undefined || item.capability.version === version)) ? 'enabled' : 'none';
}

export function placeCapability(
  document: AgentPresetDocument,
  capability: ExactCatalogRef<'capability'>,
  placement: CapabilityPlacement
): AgentPresetDocument {
  const existing = document.enabled_capabilities.find(item =>
    item.capability.id === capability.id && item.capability.version === capability.version);
  const enabled = document.enabled_capabilities.filter(item => item.capability.id !== capability.id);
  if (placement === 'enabled') enabled.push(existing ?? selection(capability));
  return { ...document, enabled_capabilities: enabled.sort((a, b) => a.capability.id.localeCompare(b.capability.id)) };
}

export function toggleSkill(
  document: AgentPresetDocument,
  skill: ExactCatalogRef<'skill'>
): AgentPresetDocument {
  const selected = document.skill_bindings.some((item) => item.id === skill.id);
  return {
    ...document,
    skill_bindings: selected
      ? document.skill_bindings.filter((item) => item.id !== skill.id)
      : [...document.skill_bindings, skill].sort((left, right) =>
          left.id.localeCompare(right.id)
        ),
  };
}

export function selectedCapabilityIds(document: AgentPresetDocument): Set<CapabilityId> {
  return new Set(
    document.enabled_capabilities.map(
      (item) => item.capability.id
    )
  );
}

export function missingSkillCapabilities(
  skill: SkillCatalogItem,
  document: AgentPresetDocument
): CapabilityId[] {
  const selected = selectedCapabilityIds(document);
  return skill.required_capabilities
    .map((item) => item.id)
    .filter((id) => !selected.has(id));
}

export function requiredResourceKinds(
  document: AgentPresetDocument,
  catalog: AgentCatalogResponse
): string[] {
  const selected = selectedCapabilityIds(document);
  const kinds = new Set<string>();
  for (const capability of catalog.capabilities) {
    if (!selected.has(capability.capability.id)) continue;
    capability.required_resource_kinds.forEach((kind) => kinds.add(kind));
  }
  return [...kinds].sort();
}
