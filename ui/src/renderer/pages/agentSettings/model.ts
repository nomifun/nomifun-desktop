import type {
  AgentPresetDocument,
  AgentPresetDraft,
  ChatRouteCandidate,
  ChatRouteRecord,
  AgentPresetEditorResponse,
  CapabilityCatalogItem,
  CapabilityId,
  OfficialPresetKey,
  OfficialPresetTemplate,
  PreviewDiagnostic,
  ResolveAgentPresetPreviewResponse,
  SaveAgentPresetRevisionResponse,
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

/**
 * The logical workspace resource identity is intentionally separate from its
 * native path.  The host supplies the latter through `typed_parameters`; the
 * editor must never reinterpret `resource_id` as a filesystem path.
 */
export const DEFAULT_WORKSPACE_RESOURCE_ID = 'workspace.default';
export const WORKSPACE_ROOT_PARAMETER = 'workspace_root';
export const KNOWLEDGE_ROOT_PARAMETER = 'knowledge_root';
export const KNOWLEDGE_NAME_PARAMETER = 'knowledge_name';

export type KnowledgeBindingSource = {
  knowledge_base_id: string;
  name: string;
  root_path: string;
};

export function bindKnowledgeBaseResource(
  binding: TypedResourceBinding,
  knowledgeBase: KnowledgeBindingSource
): TypedResourceBinding {
  return {
    ...binding,
    resource_id: knowledgeBase.knowledge_base_id,
    typed_parameters: {
      ...(binding.typed_parameters ?? {}),
      [KNOWLEDGE_ROOT_PARAMETER]: knowledgeBase.root_path,
      [KNOWLEDGE_NAME_PARAMETER]: knowledgeBase.name,
    },
  };
}

/**
 * Workspace paths are host-owned parameters, not resource identities. Keep
 * the logical binding id opaque while allowing the product picker to replace
 * only the selected path.
 */
export function bindWorkspaceResource(
  binding: TypedResourceBinding,
  workspaceRoot: string
): TypedResourceBinding {
  const root = workspaceRoot.trim();
  const typedParameters = { ...(binding.typed_parameters ?? {}) };

  if (root) {
    typedParameters[WORKSPACE_ROOT_PARAMETER] = root;
  } else {
    delete typedParameters[WORKSPACE_ROOT_PARAMETER];
  }

  return {
    ...binding,
    resource_id: root ? binding.resource_id.trim() || DEFAULT_WORKSPACE_RESOURCE_ID : '',
    typed_parameters: typedParameters,
  };
}

export const chatRouteCandidateKey = (candidate: ChatRouteCandidate): string =>
  `${candidate.model_route_id}@${candidate.model_route_revision}`;

/**
 * Reorder an exact route record after a friendly model choice. The selected
 * candidate remains byte-for-byte intact; no route id, credential ref, or
 * provider digest is inferred in the renderer.
 */
export function selectChatRouteCandidate(
  record: ChatRouteRecord | null | undefined,
  candidateKey: string
): ChatRouteRecord | null {
  if (!record) return null;
  const candidates = [record.primary, ...record.failovers];
  const selected = candidates.find(
    (candidate) => chatRouteCandidateKey(candidate) === candidateKey
  );
  if (!selected) return null;

  return {
    ...record,
    primary: selected,
    failovers: candidates.filter((candidate) => chatRouteCandidateKey(candidate) !== candidateKey),
  };
}

export interface SaveDraftRevisionPorts {
  preview(draft: AgentPresetDraft): Promise<ResolveAgentPresetPreviewResponse>;
  save(
    draft: AgentPresetDraft,
    preview: ResolveAgentPresetPreviewResponse
  ): Promise<SaveAgentPresetRevisionResponse>;
}

export async function saveDraftRevisionWithPreview(
  draft: AgentPresetDraft,
  ports: SaveDraftRevisionPorts
): Promise<{
  preview: ResolveAgentPresetPreviewResponse;
  saved: SaveAgentPresetRevisionResponse | null;
}> {
  const preview = await ports.preview(draft);
  if (!preview.can_save_revision || preview.status !== 'ready') {
    return { preview, saved: null };
  }
  return {
    preview,
    saved: await ports.save(draft, preview),
  };
}

export const selectedCapabilityCount = (document: AgentPresetDocument): number =>
  document.initial_capabilities.length + document.on_demand_capabilities.length;

export function templateCapabilityCount(template: OfficialPresetTemplate): number {
  return template.seed.initial_capabilities.length + template.seed.on_demand_capabilities.length;
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
  operations: string[],
  defaults: {
    resourceId?: string;
    typedParameters?: Record<string, string>;
  } = {}
): TypedResourceBinding => ({
  binding_id: `${resourceKind}-primary`,
  resource_kind: resourceKind,
  resource_id: defaults.resourceId ?? '',
  owner_id: ownerId,
  operations,
  typed_parameters: defaults.typedParameters ?? {},
});

export function withHostResolvedWorkspaceBinding(
  draft: AgentPresetDraft,
  hostWorkDir: string | null
): AgentPresetDraft {
  const workspaceRoot = hostWorkDir?.trim();
  if (!workspaceRoot) return draft;

  let changed = false;
  const resourceBindings = draft.document.resource_bindings.map((binding) => {
    if (binding.resource_kind !== 'workspace') return binding;

    const existingRoot = binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]?.trim();
    const effectiveRoot = existingRoot || workspaceRoot;
    if (!effectiveRoot) {
      return binding;
    }
    const nextBinding = bindWorkspaceResource(binding, effectiveRoot);
    if (
      nextBinding.resource_id === binding.resource_id &&
      nextBinding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER] ===
        binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]
    ) {
      return binding;
    }
    changed = true;
    return nextBinding;
  });

  return changed
    ? updateDocument(draft, (document) => ({
        ...document,
        resource_bindings: resourceBindings,
      }))
    : draft;
}

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

export function templateDraftForInspection(template: OfficialPresetTemplate): AgentPresetDraft {
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

export const editorDraft = (editor: AgentPresetEditorResponse): AgentPresetDraft =>
  structuredClone(editor.draft);
