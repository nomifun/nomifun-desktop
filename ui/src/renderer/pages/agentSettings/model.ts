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
export const DEFAULT_PROCESS_SESSION_RESOURCE_ID = 'process.local';
export const WORKSPACE_ROOT_PARAMETER = 'workspace_root';
export const KNOWLEDGE_ROOT_PARAMETER = 'knowledge_root';
export const KNOWLEDGE_NAME_PARAMETER = 'knowledge_name';
const HOST_MANAGED_RESOURCE_KINDS = new Set(['workspace', 'process_session']);

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

/**
 * The Nomi-core process supervisor is host-managed and scoped to the same
 * workspace as the Session. Users select the workspace; the renderer derives
 * the opaque process-session binding instead of asking for an internal ID.
 */
export function bindProcessSessionResource(
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
    resource_id: root
      ? binding.resource_id.trim() || DEFAULT_PROCESS_SESSION_RESOURCE_ID
      : '',
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

export function workspaceRootForDraft(
  draft: AgentPresetDraft,
  hostWorkDir: string | null
): string | null {
  const workspaceBinding = draft.document.resource_bindings.find(
    (binding) => binding.resource_kind === 'workspace'
  );
  return (
    workspaceBinding?.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]?.trim() ||
    hostWorkDir?.trim() ||
    null
  );
}

export function withHostResolvedWorkspaceBinding(
  draft: AgentPresetDraft,
  hostWorkDir: string | null
): AgentPresetDraft {
  const workspaceBinding = draft.document.resource_bindings.find(
    (binding) => binding.resource_kind === 'workspace'
  );
  const workspaceRoot = workspaceRootForDraft(draft, hostWorkDir) ?? '';
  if (!workspaceBinding && !workspaceRoot) return draft;

  let changed = false;
  const resourceBindings = draft.document.resource_bindings.map((binding) => {
    if (binding.resource_kind === 'process_session') {
      const nextBinding = bindProcessSessionResource(binding, workspaceRoot);
      if (
        nextBinding.resource_id === binding.resource_id &&
        nextBinding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER] ===
          binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]
      ) {
        return binding;
      }
      changed = true;
      return nextBinding;
    }
    if (binding.resource_kind !== 'workspace') return binding;

    const nextBinding = bindWorkspaceResource(binding, workspaceRoot);
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

/**
 * Materialize the host workspace and its coupled process session as soon as a
 * capability selection starts requiring them. The editor may render a host
 * default before either binding exists in the draft; persisting the same value
 * here prevents Preview and Save from sending visually selected but unbound
 * resources.
 */
export function ensureWorkspaceBinding(
  draft: AgentPresetDraft,
  hostWorkDir: string | null,
  requiredResourceKinds: string[],
  ownerId: string,
  operations: string[] = ['read', 'write', 'execute']
): AgentPresetDraft {
  let resolvedDraft = withHostResolvedWorkspaceBinding(draft, hostWorkDir);
  const root = workspaceRootForDraft(resolvedDraft, hostWorkDir);
  if (!root) return resolvedDraft;

  if (requiredResourceKinds.includes('workspace')) {
    const existing = resolvedDraft.document.resource_bindings.find(
      (binding) => binding.resource_kind === 'workspace'
    );
    const binding =
      existing ??
      defaultResourceBinding(
        'workspace',
        ownerId ||
          resolvedDraft.document.resource_bindings.find(
            (candidate) => candidate.resource_kind === 'process_session'
          )?.owner_id ||
          '',
        operations
      );
    const resolved = bindWorkspaceResource(binding, root);
    if (
      !existing ||
      existing.resource_id !== resolved.resource_id ||
      existing.typed_parameters?.[WORKSPACE_ROOT_PARAMETER] !==
        resolved.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]
    ) {
      resolvedDraft = updateResourceBinding(resolvedDraft, resolved);
    }
  }
  if (requiredResourceKinds.includes('process_session')) {
    const existing = resolvedDraft.document.resource_bindings.find(
      (binding) => binding.resource_kind === 'process_session'
    );
    const workspaceOwnerId = resolvedDraft.document.resource_bindings.find(
      (binding) => binding.resource_kind === 'workspace'
    )?.owner_id;
    const binding =
      existing ??
      defaultResourceBinding(
        'process_session',
        ownerId || workspaceOwnerId || '',
        ['execute', 'observe']
      );
    const resolved = bindProcessSessionResource(binding, root);
    if (
      !existing ||
      existing.resource_id !== resolved.resource_id ||
      existing.typed_parameters?.[WORKSPACE_ROOT_PARAMETER] !==
        resolved.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]
    ) {
      resolvedDraft = updateResourceBinding(resolvedDraft, resolved);
    }
  }
  return resolvedDraft;
}

export function resourceKindsForDraft(
  draft: AgentPresetDraft,
  capabilities: CapabilityCatalogItem[]
): string[] {
  const kinds = new Set(
    requiredResourceKinds(draft.document, {
      capabilities,
      skills: [],
      mcp_tools: [],
    })
  );
  // A host-managed process session is always rooted in a workspace. Keep the
  // workspace binding explicit even when process.exec is the only selected
  // capability and its catalog contract names only process_session.
  if (kinds.has('process_session')) kinds.add('workspace');
  return [...kinds].sort();
}

function withHostManagedResourceBindingRefs(
  draft: AgentPresetDraft,
  capabilities: CapabilityCatalogItem[]
): AgentPresetDraft {
  const capabilityById = new Map(
    capabilities.map((capability) => [capability.capability.id, capability])
  );
  const bindingById = new Map(
    draft.document.resource_bindings.map((binding) => [binding.binding_id, binding])
  );
  const usableBindingByKind = new Map<string, TypedResourceBinding>();
  for (const binding of draft.document.resource_bindings) {
    if (
      HOST_MANAGED_RESOURCE_KINDS.has(binding.resource_kind) &&
      binding.resource_id.trim() &&
      binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]?.trim() &&
      !usableBindingByKind.has(binding.resource_kind)
    ) {
      usableBindingByKind.set(binding.resource_kind, binding);
    }
  }

  let changed = false;
  const bindSelections = (selections: AgentPresetDocument['initial_capabilities']) =>
    selections.map((selection) => {
      const capability = capabilityById.get(selection.capability.id);
      if (!capability) return selection;

      const requiredHostKinds = capability.required_resource_kinds.filter((kind) =>
        HOST_MANAGED_RESOURCE_KINDS.has(kind)
      );
      const currentRefs = selection.resource_binding_refs ?? [];
      const nextRefs = [
        ...currentRefs.filter((bindingId) => {
          const kind = bindingById.get(bindingId)?.resource_kind;
          return kind == null || !HOST_MANAGED_RESOURCE_KINDS.has(kind);
        }),
        ...requiredHostKinds.flatMap((kind) => {
          const binding = usableBindingByKind.get(kind);
          return binding ? [binding.binding_id] : [];
        }),
      ]
        .filter((bindingId, index, refs) => refs.indexOf(bindingId) === index)
        .sort();

      if (
        currentRefs.length === nextRefs.length &&
        currentRefs.every((bindingId, index) => bindingId === nextRefs[index])
      ) {
        return selection;
      }
      changed = true;
      return { ...selection, resource_binding_refs: nextRefs };
    });

  const initialCapabilities = bindSelections(draft.document.initial_capabilities);
  const onDemandCapabilities = bindSelections(draft.document.on_demand_capabilities);
  return changed
    ? updateDocument(draft, (document) => ({
        ...document,
        initial_capabilities: initialCapabilities,
        on_demand_capabilities: onDemandCapabilities,
      }))
    : draft;
}

/**
 * Complete the renderer-owned portion of host resource resolution. This is
 * intentionally shared by editing, Preview, Save, and Test so no operation can
 * send a visually selected workspace with a missing/stale process binding.
 */
export function resolveHostManagedResourceBindings(
  draft: AgentPresetDraft,
  hostWorkDir: string | null,
  capabilities: CapabilityCatalogItem[],
  ownerId: string
): AgentPresetDraft {
  const withBindings = ensureWorkspaceBinding(
    draft,
    hostWorkDir,
    resourceKindsForDraft(draft, capabilities),
    ownerId
  );
  return withHostManagedResourceBindingRefs(withBindings, capabilities);
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

/**
 * Keep transport/runtime details out of the product surface.  The canonical
 * APIs intentionally return machine-readable error codes and, in some cases,
 * diagnostic payloads.  Those are useful to logs and tests, but rendering the
 * thrown Error directly leaks endpoint paths, UUIDs, digests, and JSON into a
 * normal user flow.
 */
export type AgentUiOperation =
  | 'load'
  | 'open'
  | 'create'
  | 'fork'
  | 'preview'
  | 'save'
  | 'test'
  | 'session-load'
  | 'turn'
  | 'session-fork'
  | 'session-delete'
  | 'resources';

export type AgentUiErrorKind =
  | 'route-unavailable'
  | 'network'
  | 'timeout'
  | 'session-deleted'
  | 'session-not-found'
  | 'snapshot-unavailable'
  | 'resource'
  | 'model'
  | 'conflict'
  | 'runtime'
  | 'unknown';

type ErrorShape = {
  code?: unknown;
  status?: unknown;
  kind?: unknown;
};

const errorShape = (error: unknown): ErrorShape | null =>
  error && typeof error === 'object' ? (error as ErrorShape) : null;

const errorCode = (error: unknown): string => {
  const code = errorShape(error)?.code;
  return typeof code === 'string' ? code.toUpperCase() : '';
};

const errorStatus = (error: unknown): number | null => {
  const status = errorShape(error)?.status;
  return typeof status === 'number' && Number.isFinite(status) ? status : null;
};

const errorKind = (error: unknown): string => {
  const kind = errorShape(error)?.kind;
  return typeof kind === 'string' ? kind.toLowerCase() : '';
};

export function classifyAgentUiError(
  error: unknown,
  operation: AgentUiOperation
): AgentUiErrorKind {
  const code = errorCode(error);
  const status = errorStatus(error);
  const kind = errorKind(error);

  if (code === 'SESSION_DELETED') return 'session-deleted';
  if (code === 'SESSION_NOT_FOUND' || code === 'REMOTE_SESSION_NOT_FOUND') {
    return 'session-not-found';
  }
  if (code === 'SNAPSHOT_EXECUTOR_UNAVAILABLE') return 'snapshot-unavailable';
  if (
    code === 'PRESET_RESOURCE_NOT_BOUND' ||
    code === 'RESOURCE_OWNER_MISMATCH' ||
    code === 'CAPABILITY_RESOURCE_NOT_BOUND'
  ) {
    return 'resource';
  }
  if (
    code === 'MODEL_ROUTE_RECORD_INVALID' ||
    code === 'MODEL_ROUTE_NOT_FOUND' ||
    code === 'CAPABILITY_NOT_MATERIALIZED' ||
    code === 'CAPABILITY_UNAVAILABLE' ||
    code === 'CAPABILITY_UNAVAILABLE_ON_PLATFORM'
  ) {
    return 'model';
  }
  if (
    code === 'PRESET_REVISION_DIGEST_MISMATCH' ||
    code === 'IDEMPOTENCY_CONFLICT' ||
    status === 409
  ) {
    return 'conflict';
  }
  if (
    code === 'AGENT_PLATFORM_RUNTIME_FAILED' ||
    code === 'AGENT_PLATFORM_INTERNAL' ||
    code === 'REMOTE_OPEN_FAILED'
  ) {
    return 'runtime';
  }
  if (kind === 'timeout') return 'timeout';
  if (kind === 'network') return 'network';

  // A 404/405 from a canonical endpoint is normally an unmounted route, not a
  // malformed user action.  Preserve the more specific Session codes above.
  if (
    status === 404 ||
    status === 405 ||
    code === 'NON_JSON_RESPONSE' ||
    code === 'ROUTE_NOT_FOUND'
  ) {
    return 'route-unavailable';
  }

  // A missing code on a load operation is commonly an HTML/404 response from
  // an older Nomi-core assembly.  Do not show that response body to the user.
  if (operation === 'load' && status == null) return 'route-unavailable';
  return 'unknown';
}

export function agentUiErrorMessage(
  error: unknown,
  operation: AgentUiOperation
): string {
  switch (classifyAgentUiError(error, operation)) {
    case 'route-unavailable':
      return 'The current Nomi-core build does not expose the canonical Agent workflow. Start the matching service or update the application, then retry.';
    case 'network':
      return 'The Nomi-core service could not be reached. Check that the desktop service is running, then retry.';
    case 'timeout':
      return 'The Nomi-core service did not respond before the deadline. Retry once; if the result is uncertain, inspect the existing Session before submitting again.';
    case 'session-deleted':
      return 'This Session was deleted and can no longer be continued.';
    case 'session-not-found':
      return 'This Session is no longer available. Return to Agent Settings and choose another setup.';
    case 'snapshot-unavailable':
      return 'This saved setup cannot run on the current runtime. Its history is read-only; create a new Session from the current setup.';
    case 'resource':
      return 'Select every required resource before saving or testing this setup.';
    case 'model':
      return 'Choose an available Chat model before saving or testing this setup.';
    case 'conflict':
      return 'This setup changed elsewhere. Reload it before saving again.';
    case 'runtime':
      return 'Nomi-core could not start this operation. Check the selected model and resources, then retry.';
    case 'unknown':
    default:
      return 'The Agent operation could not be completed. Review the selected model and resources, then retry.';
  }
}

/**
 * Preview diagnostics are deliberately reduced to product language.  The
 * backend diagnostic code/subject/details remain available in the response
 * for non-UI callers, but never become a visible error payload.
 */
export function previewDiagnosticMessage(
  diagnostic: PreviewDiagnostic
): string {
  switch (diagnostic.code.toUpperCase()) {
    case 'PRESET_RESOURCE_NOT_BOUND':
    case 'RESOURCE_OWNER_MISMATCH':
      return 'Select every required resource before continuing.';
    case 'MODEL_ROUTE_RECORD_INVALID':
    case 'MODEL_ROUTE_NOT_FOUND':
      return 'Choose an available Chat model before continuing.';
    case 'CAPABILITY_NOT_MATERIALIZED':
    case 'CAPABILITY_UNAVAILABLE':
    case 'CAPABILITY_UNAVAILABLE_ON_PLATFORM':
      return 'One selected capability is unavailable on this installation.';
    case 'PRESET_REVISION_DIGEST_MISMATCH':
      return 'This setup changed while it was open. Reload it before saving.';
    case 'SNAPSHOT_EXECUTOR_UNAVAILABLE':
      return 'This setup is read-only on the current runtime. Fork a new Session to continue.';
    case 'ROLE_COVERAGE_INCOMPLETE':
    case 'CODING_CODEX_NATIVE_INCOMPLETE':
      return 'This setup requires a runtime feature that is not available on this host.';
    default:
      return diagnostic.severity === 'warning'
        ? 'An optional part of this setup is unavailable on the current host.'
        : 'This setup cannot be executed on the current host.';
  }
}
