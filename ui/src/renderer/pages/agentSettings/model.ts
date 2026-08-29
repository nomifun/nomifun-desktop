import type {
  AgentPresetDocument,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  CapabilityCatalogItem,
  CapabilityId,
  OfficialPresetKey,
  OfficialPresetTemplate,
  PreviewDiagnostic,
  ResolveAgentPresetPreviewResponse,
  TypedResourceBinding,
} from '@/common/types/agentPlatform';
import {
  asCapabilityId,
  createEmptyAgentPresetDocument,
  requiredResourceKinds,
  upsertResourceBinding,
} from '@/common/types/agentPlatform';

export const TEMPLATE_I18N_PATH: Record<OfficialPresetKey, string> = {
  'chat.minimal': 'chat.minimal',
  'assistant.general': 'assistant.general',
  'coding.codex': 'coding.codex',
  'companion.default': 'companion.default',
  'robot.default': 'robot.default',
  'customer-service.default': 'customerService.default',
  'creative-studio.default': 'creativeStudio.default',
};

export const selectedCapabilityCount = (document: AgentPresetDocument): number =>
  document.initial_capabilities.length + document.on_demand_capabilities.length;

export function templateCapabilityCount(template: OfficialPresetTemplate): number {
  return (
    template.seed.initial_capabilities.length + template.seed.on_demand_capabilities.length
  );
}

export function updateDocument(
  draft: AgentPresetDraft,
  transform: (document: AgentPresetDocument) => AgentPresetDocument
): AgentPresetDraft {
  return { ...draft, document: transform(draft.document) };
}

export function updateResourceBinding(
  draft: AgentPresetDraft,
  binding: TypedResourceBinding
): AgentPresetDraft {
  return updateDocument(draft, (document) => ({
    ...document,
    resource_bindings: upsertResourceBinding(document.resource_bindings, binding),
  }));
}

export function removeResourceBinding(
  draft: AgentPresetDraft,
  bindingId: string
): AgentPresetDraft {
  return updateDocument(draft, (document) => ({
    ...document,
    resource_bindings: document.resource_bindings.filter(
      (binding) => binding.binding_id !== bindingId
    ),
  }));
}

export const defaultResourceBinding = (
  resourceKind: string,
  ownerId: string,
  operations: string[]
): TypedResourceBinding => ({
  binding_id: `${resourceKind}-primary`,
  resource_kind: resourceKind,
  resource_id: '',
  owner_id: ownerId,
  operations,
  typed_parameters: {},
});

export function resourceKindsForDraft(
  draft: AgentPresetDraft,
  capabilities: CapabilityCatalogItem[]
): string[] {
  return requiredResourceKinds(draft.document, {
    capabilities,
    skills: [],
    mcp_tools: [],
  });
}

export function templateDraftForInspection(
  template: OfficialPresetTemplate
): AgentPresetDraft {
  const document = createEmptyAgentPresetDocument();
  return {
    preset_id: '' as AgentPresetDraft['preset_id'],
    display_name: template.template_key,
    source_template_key: template.template_key,
    document: {
      ...document,
      initial_capabilities: template.seed.initial_capabilities.map((capability) => ({
        capability,
        required: true,
        exposure: 'advertised',
        config: {},
      })),
      on_demand_capabilities: template.seed.on_demand_capabilities.map((capability) => ({
        capability,
        required: true,
        exposure: 'discoverable',
        config: {},
      })),
      skill_bindings: template.seed.skill_bindings,
    },
  };
}

export const previewPrimaryDiagnostic = (
  preview: ResolveAgentPresetPreviewResponse | null
): PreviewDiagnostic | null =>
  preview?.diagnostics.find((diagnostic) => diagnostic.severity === 'error') ??
  preview?.diagnostics[0] ??
  null;

export const capabilityById = (
  capabilities: CapabilityCatalogItem[]
): Map<CapabilityId, CapabilityCatalogItem> =>
  new Map(capabilities.map((capability) => [capability.capability.id, capability]));

export const selectedCapabilityIds = (draft: AgentPresetDraft): CapabilityId[] =>
  [...draft.document.initial_capabilities, ...draft.document.on_demand_capabilities].map(
    (selection) => asCapabilityId(selection.capability.id)
  );

export const editorDraft = (
  editor: AgentPresetEditorResponse
): AgentPresetDraft => structuredClone(editor.draft);
