import type {
  AgentPresetDocument,
  AgentPresetDraft,
  CapabilityModuleCatalogItem,
  ChatRouteCandidate,
  ChatRouteRecord,
  ExactCatalogRef,
  OfficialPresetKey,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';

export const TEMPLATE_I18N_PATH: Record<OfficialPresetKey, string> = {
  'chat.minimal': 'chat.minimal',
  'assistant.general': 'assistant.general',
  'coding.codex': 'coding.codex',
  'companion.default': 'companion.default',
  'customer-service.default': 'customerService.default',
  'creative-studio.default': 'creativeStudio.default',
};

/** One explicit authoring detour in browser history; never model routes, credentials, or Runtime selection. */
export type AgentEditingDocument = Omit<
  AgentPresetDocument,
  'model_route_refs' | 'chat_route_records'
>;
export type TemplateEditingState = {
  displayName: string;
  document: AgentEditingDocument;
  activeTab: string;
};
export type AgentEditorReturn = { version: 1; search: string } & (
  | { kind: 'template'; templateKey: OfficialPresetKey; editing: TemplateEditingState }
  | { kind: 'preset'; draft: Omit<AgentPresetDraft, 'document'> & { document: AgentEditingDocument } }
);

export function editingDocument(document: AgentPresetDocument): AgentEditingDocument {
  const {
    model_route_refs: _routes,
    chat_route_records: _records,
    ...editing
  } = document;
  return structuredClone(editing);
}

export function agentEditorReturn(state: unknown, search: string): AgentEditorReturn | null {
  const value = state && typeof state === 'object'
    ? (state as { agentEditorReturn?: AgentEditorReturn }).agentEditorReturn
    : null;
  if (!value || value.version !== 1 || value.search !== search) return null;
  const params = new URLSearchParams(search);
  if (
    value.kind === 'template' &&
    value.editing &&
    typeof value.editing.displayName === 'string' &&
    Array.isArray(value.editing.document?.enabled_capabilities) &&
    (!params.has('template') || params.get('template') === value.templateKey) &&
    !params.has('preset')
  ) return value;
  if (
    value.kind === 'preset' &&
    value.draft &&
    typeof value.draft.preset_id === 'string' &&
    Array.isArray(value.draft.document?.enabled_capabilities) &&
    (!params.has('preset') || params.get('preset') === value.draft.preset_id) &&
    !params.has('template')
  ) return value;
  return null;
}

export const isAgentModelConfigurationMissing = (error: unknown): boolean =>
  errorCode(error) === 'MODEL_ROUTE_NOT_CONFIGURED';

export const chatRouteCandidateKey = (candidate: ChatRouteCandidate): string =>
  `${candidate.model_route_id}@${candidate.model_route_revision}`;

/** Reorder an exact route record after a friendly model choice; the candidate remains byte-for-byte intact. */
export function selectChatRouteCandidate(
  record: ChatRouteRecord | null | undefined,
  candidateKey: string
): ChatRouteRecord | null {
  if (!record) return null;
  const candidates = [record.primary, ...record.failovers];
  const selected = candidates.find((candidate) => chatRouteCandidateKey(candidate) === candidateKey);
  if (!selected) return null;
  return {
    ...record,
    primary: selected,
    failovers: candidates.filter((candidate) => chatRouteCandidateKey(candidate) !== candidateKey),
  };
}

export function templateModuleCount(template: OfficialPresetTemplate): number {
  return template.seed.enabled_capabilities.length;
}

export function updateDocument(
  draft: AgentPresetDraft,
  transform: (document: AgentPresetDocument) => AgentPresetDocument
): AgentPresetDraft {
  return { ...draft, document: transform(draft.document) };
}

export const capabilityReferenceKey = (
  reference: ExactCatalogRef<'capability'>
): string => `${reference.id}@${reference.version}`;

export const RESOURCE_KIND_I18N_KEYS: Readonly<Record<string, string>> = {
  asset_library: 'assetLibrary',
  browser: 'browser',
  canvas: 'canvas',
  channel: 'channel',
  companion: 'companion',
  companion_memory: 'companionMemory',
  computer: 'computer',
  customer: 'customer',
  generation_provider: 'generationProvider',
  knowledge_base: 'knowledgeBase',
  mcp_server: 'mcpConnection',
  plugin: 'pluginRuntime',
  process_session: 'processSession',
  project_memory: 'projectMemory',
  robot: 'robot',
  ssh_host: 'sshHost',
  terminal: 'terminal',
  workspace: 'workspace',
};

export const humanizeResourceKind = (resourceKind: string): string =>
  resourceKind
    .split('_')
    .filter(Boolean)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(' ');

/** Product-localized copy keys for the vNext Module identities only. */
export const MODULE_I18N_KEYS: Readonly<Record<string, string>> = {
  'agent.collaboration': 'agentCollaboration',
  'agent.tool-discovery': 'toolDiscovery',
  'automation.schedule': 'automationSchedule',
  browser: 'browser',
  'channel.messaging': 'channelMessaging',
  companion: 'companion',
  'companion.memory': 'companionMemory',
  computer: 'computer',
  'creation.media': 'creationMedia',
  'creative.workshop': 'creativeWorkshop',
  'customer.service': 'customerService',
  knowledge: 'knowledge',
  office: 'office',
  'plugin.development': 'pluginDevelopment',
  'project.memory': 'projectMemory',
  requirements: 'requirements',
  robot: 'robot',
  ssh: 'ssh',
  'web.research': 'webResearch',
  'workspace.artifacts': 'workspaceArtifacts',
  'workspace.files': 'workspaceFiles',
  'workspace.process': 'workspaceProcess',
  'workspace.vcs': 'workspaceVcs',
};

export const moduleI18nKey = (moduleId: string): string | undefined => MODULE_I18N_KEYS[moduleId];

export function moduleMatchesSearch(
  module: CapabilityModuleCatalogItem,
  query: string,
  localizedName: string,
  localizedDescription: string
): boolean {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return true;
  return [
    localizedName,
    localizedDescription,
    module.display_name,
    module.description,
    module.module.id,
    module.source_package.id,
    ...module.required_resource_kinds,
    ...module.actions.flatMap((action) => [action.action_id, action.effect_class]),
  ].join(' ').toLocaleLowerCase().includes(normalized);
}

export function actionFallbackName(actionId: string): string {
  const leaf = actionId.includes('/') ? actionId.slice(actionId.indexOf('/') + 1) : actionId;
  return leaf
    .split(/[._-]+/)
    .filter(Boolean)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(' ');
}

export { capabilityPlacement, placeCapability } from '@/common/types/agentPlatform';

export type AgentUiOperation =
  | 'load'
  | 'open'
  | 'create'
  | 'delete'
  | 'fork'
  | 'save'
  | 'session-load'
  | 'turn'
  | 'session-fork'
  | 'session-delete';

export type AgentUiErrorKind =
  | 'route-unavailable'
  | 'network'
  | 'timeout'
  | 'preset-not-found'
  | 'session-deleted'
  | 'session-not-found'
  | 'snapshot-unavailable'
  | 'resource'
  | 'capability'
  | 'model'
  | 'conflict'
  | 'runtime'
  | 'unknown';

type ErrorShape = { code?: unknown; status?: unknown; kind?: unknown; details?: unknown };
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
  if (code === 'AGENT_PRESET_NOT_FOUND') return 'preset-not-found';
  if (code === 'SESSION_DELETED') return 'session-deleted';
  if (code === 'SESSION_NOT_FOUND' || code === 'REMOTE_SESSION_NOT_FOUND') return 'session-not-found';
  if (code === 'SNAPSHOT_EXECUTOR_UNAVAILABLE') return 'snapshot-unavailable';
  if (
    code === 'PRESET_RESOURCE_NOT_BOUND' ||
    code === 'RESOURCE_OWNER_MISMATCH' ||
    code === 'CAPABILITY_RESOURCE_NOT_BOUND'
  ) return 'resource';
  if (
    code === 'MODEL_ROUTE_NOT_CONFIGURED' ||
    code === 'MODEL_ROUTE_RECORD_INVALID' ||
    code === 'MODEL_ROUTE_NOT_FOUND'
  ) return 'model';
  if (
    code === 'CAPABILITY_NOT_MATERIALIZED' ||
    code === 'CAPABILITY_UNAVAILABLE' ||
    code === 'CAPABILITY_UNAVAILABLE_ON_PLATFORM' ||
    code === 'CAPABILITY_ACTION_NOT_MATERIALIZED' ||
    code === 'CAPABILITY_NOT_AUTHORABLE' ||
    code === 'CAPABILITY_CONTRACT_MISMATCH'
  ) return 'capability';
  if (
    code === 'PRESET_REVISION_DIGEST_MISMATCH' ||
    code === 'IDEMPOTENCY_CONFLICT' ||
    status === 409
  ) return 'conflict';
  if (
    code === 'AGENT_PLATFORM_RUNTIME_FAILED' ||
    code === 'AGENT_PLATFORM_INTERNAL' ||
    code === 'REMOTE_OPEN_FAILED'
  ) return 'runtime';
  if (kind === 'timeout') return 'timeout';
  if (kind === 'network') return 'network';
  if (status === 404 || status === 405 || code === 'NON_JSON_RESPONSE' || code === 'ROUTE_NOT_FOUND') {
    return 'route-unavailable';
  }
  if (operation === 'load' && status == null) return 'route-unavailable';
  return 'unknown';
}

export function agentUiErrorKey(error: unknown, operation: AgentUiOperation): string {
  switch (classifyAgentUiError(error, operation)) {
    case 'route-unavailable':
      return 'agentSettings.errors.routeUnavailable';
    case 'network':
      return 'agentSettings.errors.network';
    case 'timeout':
      return 'agentSettings.errors.timeout';
    case 'preset-not-found':
      return 'agentSettings.errors.presetNotFound';
    case 'session-deleted':
      return 'agentSettings.errors.sessionDeleted';
    case 'session-not-found':
      return 'agentSettings.errors.sessionNotFound';
    case 'snapshot-unavailable':
      return 'agentSettings.errors.snapshotUnavailable';
    case 'resource':
      return 'agentSettings.errors.resource';
    case 'capability':
      return 'agentSettings.errors.capability';
    case 'model':
      return 'agentSettings.errors.model';
    case 'conflict':
      return 'agentSettings.errors.conflict';
    case 'runtime':
      return 'agentSettings.errors.runtime';
    case 'unknown':
    default:
      return 'agentSettings.errors.unknown';
  }
}

export function agentUiErrorSubjects(error: unknown): string[] {
  const details = errorShape(error)?.details;
  if (!details || typeof details !== 'object') return [];
  const diagnostics = (details as { diagnostics?: unknown }).diagnostics;
  if (!Array.isArray(diagnostics)) return [];
  return [...new Set(diagnostics.flatMap((diagnostic) => {
    if (!diagnostic || typeof diagnostic !== 'object') return [];
    const subject = (diagnostic as { subject?: unknown }).subject;
    return typeof subject === 'string' && subject.trim() ? [subject.trim()] : [];
  }))].slice(0, 8);
}
