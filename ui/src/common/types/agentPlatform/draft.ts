import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  AgentPresetDraft,
  CapabilityId,
  CapabilitySelection,
  ExactCatalogRef,
  OfficialPresetTemplate,
  SkillCatalogItem,
  TypedResourceBinding,
} from './contracts';

const compareKey = (value: unknown): string => JSON.stringify(value);

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
  surfaces: ['desktop', 'remote', 'web'],
  model_route_refs: {},
  chat_route_records: {},
  initial_capabilities: [],
  on_demand_capabilities: [],
  skill_bindings: [],
  resource_bindings: [],
  persona: '',
  instructions: '',
  context_policy: {
    max_system_tokens: 12_000,
    max_dynamic_context_tokens: 16_000,
    max_catalog_tokens: 3_000,
  },
  execution_constraints: {
    max_active_capabilities: 64,
    max_advertised_tools: 48,
    max_runtime_rebuilds: 4,
  },
  runtime_budget: {
    max_context_tokens: 32_000,
    max_tool_calls_per_turn: 64,
  },
});

const selection = (
  capability: ExactCatalogRef<'capability'>,
  exposure: CapabilitySelection['exposure']
): CapabilitySelection => ({
  capability,
  required: true,
  exposure,
  action_allowlist: [],
  resource_binding_refs: [],
  destination_constraints: [],
  config: {},
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
    initial_capabilities: template.seed.initial_capabilities.map((item) =>
      selection(item, 'advertised')
    ),
    on_demand_capabilities: template.seed.on_demand_capabilities.map((item) =>
      selection(item, 'discoverable')
    ),
    skill_bindings: template.seed.skill_bindings,
  },
});

export const cloneDraft = (draft: AgentPresetDraft): AgentPresetDraft => {
  const cloned = structuredClone(draft);
  cloned.document.chat_route_records ??= {};
  return cloned;
};

export type CapabilityPlacement = 'initial' | 'on_demand' | 'none';

export function capabilityPlacement(
  document: AgentPresetDocument,
  capabilityId: CapabilityId
): CapabilityPlacement {
  if (document.initial_capabilities.some((item) => item.capability.id === capabilityId)) {
    return 'initial';
  }
  if (document.on_demand_capabilities.some((item) => item.capability.id === capabilityId)) {
    return 'on_demand';
  }
  return 'none';
}

export function placeCapability(
  document: AgentPresetDocument,
  capability: ExactCatalogRef<'capability'>,
  placement: CapabilityPlacement
): AgentPresetDocument {
  const without = (items: CapabilitySelection[]) =>
    items.filter((item) => item.capability.id !== capability.id);
  const initial = without(document.initial_capabilities);
  const onDemand = without(document.on_demand_capabilities);
  if (placement === 'initial') initial.push(selection(capability, 'advertised'));
  if (placement === 'on_demand') onDemand.push(selection(capability, 'discoverable'));
  return {
    ...document,
    initial_capabilities: initial.sort((left, right) =>
      left.capability.id.localeCompare(right.capability.id)
    ),
    on_demand_capabilities: onDemand.sort((left, right) =>
      left.capability.id.localeCompare(right.capability.id)
    ),
  };
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
    [...document.initial_capabilities, ...document.on_demand_capabilities].map(
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

export function upsertResourceBinding(
  bindings: TypedResourceBinding[],
  binding: TypedResourceBinding
): TypedResourceBinding[] {
  return [...bindings.filter((item) => item.binding_id !== binding.binding_id), binding].sort(
    (left, right) => compareKey(left.binding_id).localeCompare(compareKey(right.binding_id))
  );
}
