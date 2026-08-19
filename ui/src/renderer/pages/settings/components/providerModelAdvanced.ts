/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import { MODEL_TRAIT_ORDER } from '@/common/modelCapabilities';
import type {
  EndpointRootShape,
  ModelProtocolManifestResponse,
  ProtocolDescriptor,
  ProtocolEndpointDescriptor,
  ProtocolRecommendation,
} from '@/common/types/provider/modelProtocolManifest';
import type { ProviderModelCapabilityInput as CanonicalProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import type { ProviderConnectionInput as CanonicalProviderConnectionInput } from '@/common/types/provider/providerConnection';

/** The endpoint fields owned by a task capability on the wire. */
export const CAPABILITY_ENDPOINT_FIELDS = [
  'endpoint',
  'poll_endpoint',
  'content_endpoint',
  'realtime_endpoint',
] as const;

export type CapabilityEndpointField = (typeof CAPABILITY_ENDPOINT_FIELDS)[number];

export const isCapabilityEndpointField = (value: string): value is CapabilityEndpointField =>
  (CAPABILITY_ENDPOINT_FIELDS as readonly string[]).includes(value);

/** UI code consumes the backend-owned manifest types without redefining them. */
export type CapabilityEndpointDescriptor = ProtocolEndpointDescriptor;
export type CapabilityProtocolDescriptor = ProtocolDescriptor;
export type CapabilityProtocolRecommendation = ProtocolRecommendation;
export type ModelProtocolManifest = ModelProtocolManifestResponse;
export type CapabilityRootShape = EndpointRootShape;

export type ModelProtocolManifestMap = Partial<Record<ModelTask, ModelProtocolManifest>>;

/** Editable, task-scoped capability. No transport value is stored in raw JSON. */
export interface ModelCapabilityDraft {
  task: ModelTask;
  traits: ModelTrait[];
  protocol: string;
  connectionRole: string;
  baseUrlOverride: string;
  endpoint: string;
  pollEndpoint: string;
  contentEndpoint: string;
  realtimeEndpoint: string;
  allowCrossOriginCredentials: boolean;
  providerParamsJson: string;
  contextLimit?: number;
}

export interface ModelDefinitionDraft {
  model: string;
  capabilities: ModelCapabilityDraft[];
}

export interface CatalogCapabilitySuggestion {
  model: string;
  tasks: ModelTask[];
  traits: ModelTrait[];
}

export type ProviderModelCapabilityInput = CanonicalProviderModelCapabilityInput;
export type ProviderConnectionInput = CanonicalProviderConnectionInput;

/** Persisted connection metadata used while resolving a capability. */
export interface ProviderConnectionDescriptor {
  role: string;
  base_url: string;
  auth_scheme: string;
}

export type CapabilityValidationError =
  | 'model_required'
  | 'duplicate_model'
  | 'capability_required'
  | 'manifest_loading'
  | 'manifest_unavailable'
  | 'protocol_required'
  | 'protocol_not_registered'
  | 'auth_scheme_incompatible'
  | 'connection_role_required'
  | 'connection_missing'
  | 'base_url_required'
  | 'cross_origin_consent_required'
  | 'invalid_provider_params';

export interface CapabilityValidationResult {
  valid: boolean;
  errors: Array<{ task?: ModelTask; code: CapabilityValidationError }>;
}

const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

export const normalizeModelId = (value: string): string => value.trim();

/** Model ids are case-sensitive; only an exact normalized id is a duplicate. */
export const isDuplicateModelId = (value: string, existing: readonly string[]): boolean => {
  const normalized = normalizeModelId(value);
  return normalized.length > 0 && existing.some((candidate) => normalizeModelId(candidate) === normalized);
};

export const emptyCapabilityDraft = (task: ModelTask): ModelCapabilityDraft => ({
  task,
  traits: [],
  protocol: '',
  connectionRole: 'default',
  baseUrlOverride: '',
  endpoint: '',
  pollEndpoint: '',
  contentEndpoint: '',
  realtimeEndpoint: '',
  allowCrossOriginCredentials: false,
  providerParamsJson: '',
  contextLimit: undefined,
});

export const capabilityDraftFromResponse = (capability: {
  task: ModelTask;
  traits?: ModelTrait[];
  protocol: string;
  connection_role: string;
  base_url_override?: string;
  endpoint?: string;
  poll_endpoint?: string;
  content_endpoint?: string;
  realtime_endpoint?: string;
  allow_cross_origin_credentials?: boolean;
  provider_params?: unknown;
  context_limit?: number;
}): ModelCapabilityDraft => ({
  task: capability.task,
  traits: capability.traits ?? [],
  protocol: capability.protocol,
  connectionRole: capability.connection_role,
  baseUrlOverride: capability.base_url_override ?? '',
  endpoint: capability.endpoint ?? '',
  pollEndpoint: capability.poll_endpoint ?? '',
  contentEndpoint: capability.content_endpoint ?? '',
  realtimeEndpoint: capability.realtime_endpoint ?? '',
  allowCrossOriginCredentials: capability.allow_cross_origin_credentials ?? false,
  providerParamsJson:
    isPlainObject(capability.provider_params) && Object.keys(capability.provider_params).length > 0
      ? JSON.stringify(capability.provider_params, null, 2)
      : '',
  contextLimit: capability.context_limit,
});

/** Append one task without disturbing any existing task draft. */
export const addCapabilityTask = (
  capabilities: readonly ModelCapabilityDraft[],
  task: ModelTask
): ModelCapabilityDraft[] =>
  capabilities.some((capability) => capability.task === task)
    ? [...capabilities]
    : [...capabilities, emptyCapabilityDraft(task)];

/** Remove exactly one task while preserving every remaining task draft. */
export const removeCapabilityTask = (
  capabilities: readonly ModelCapabilityDraft[],
  task: ModelTask
): ModelCapabilityDraft[] => capabilities.filter((capability) => capability.task !== task);

const CATALOG_TRAITS_BY_TASK: Readonly<Record<ModelTask, readonly ModelTrait[]>> = {
  chat: [
    'vision_input',
    'video_input',
    'audio_input',
    'audio_output',
    'streaming',
    'function_calling',
    'reasoning',
    'web_search',
  ],
  realtime_conversation: ['audio_input', 'audio_output', 'realtime', 'streaming'],
  image_generation: [],
  image_edit: [],
  video_generation: [],
  speech_synthesis: [],
  speech_recognition: [],
  embedding: [],
  rerank: [],
};

/** AutoComplete option clicks are committed by onSelect, never by the preceding onChange. */
export const resolveModelInputChange = (model: string, option?: unknown): string | undefined =>
  option === undefined ? model : undefined;

/** Directory entries without an explicit task are never treated as universal. */
export const catalogSuggestionsForTask = <T extends { tasks: readonly ModelTask[] }>(
  suggestions: readonly T[],
  task: ModelTask | undefined
): T[] => (task ? suggestions.filter((suggestion) => suggestion.tasks.includes(task)) : []);

/**
 * A catalog choice fills only the type the user selected first. Other catalog
 * tasks remain available through the explicit "add another task" flow.
 */
export const applyCatalogSuggestionForTask = (
  _definition: ModelDefinitionDraft,
  suggestion: CatalogCapabilitySuggestion,
  task: ModelTask
): ModelDefinitionDraft => ({
  model: suggestion.model,
  capabilities: [
    {
      ...emptyCapabilityDraft(task),
      traits: suggestion.tasks.includes(task)
        ? MODEL_TRAIT_ORDER.filter(
            (trait) => suggestion.traits.includes(trait) && CATALOG_TRAITS_BY_TASK[task].includes(trait)
          )
        : [],
    },
  ],
});

/** Switching an existing primary type starts a clean task draft without leaking transport overrides. */
export const changePrimaryModelTask = (
  definition: ModelDefinitionDraft,
  task: ModelTask
): ModelDefinitionDraft => ({
  model: definition.capabilities.length === 0 ? definition.model : '',
  capabilities: [emptyCapabilityDraft(task)],
});

/** Fill only blank fields from the backend's provider × task recommendation. */
export const reconcileCapabilityRecommendations = (
  capabilities: readonly ModelCapabilityDraft[],
  manifests: ModelProtocolManifestMap
): ModelCapabilityDraft[] =>
  capabilities.map((capability) => {
    const recommendation = manifests[capability.task]?.recommendation;
    if (!recommendation) return capability;
    const untouchedProtocol = !capability.protocol;
    const protocol = capability.protocol || recommendation.protocol_id;
    const connectionRole = untouchedProtocol
      ? recommendation.connection_role || 'default'
      : capability.connectionRole || recommendation.connection_role || 'default';
    const baseUrlOverride =
      !capability.baseUrlOverride &&
      connectionRole === 'default' &&
      recommendation.base_url_override_required &&
      recommendation.default_base_url
        ? recommendation.default_base_url
        : capability.baseUrlOverride;
    return protocol === capability.protocol &&
      connectionRole === capability.connectionRole &&
      baseUrlOverride === capability.baseUrlOverride
      ? capability
      : { ...capability, protocol, connectionRole, baseUrlOverride };
  });

/**
 * Switch protocols as one atomic edit. Transport-owned fields from the old
 * adapter must never leak into the new adapter's request.
 */
export const changeCapabilityProtocol = (
  capability: ModelCapabilityDraft,
  protocol: string,
  manifest?: ModelProtocolManifest
): ModelCapabilityDraft => {
  const normalizedProtocol = protocol.trim();
  if (normalizedProtocol === capability.protocol.trim()) return capability;
  const recommendation = manifest?.recommendation;
  const isRecommendation = recommendation?.protocol_id === normalizedProtocol;
  const connectionRole = isRecommendation ? recommendation.connection_role || 'default' : 'default';
  const baseUrlOverride =
    isRecommendation &&
    connectionRole === 'default' &&
    recommendation.base_url_override_required &&
    recommendation.default_base_url
      ? recommendation.default_base_url
      : '';

  return {
    ...capability,
    protocol: normalizedProtocol,
    connectionRole,
    baseUrlOverride,
    endpoint: '',
    pollEndpoint: '',
    contentEndpoint: '',
    realtimeEndpoint: '',
    allowCrossOriginCredentials: false,
    providerParamsJson: '',
  };
};

export const protocolDescriptorForDraft = (
  capability: ModelCapabilityDraft,
  manifest?: ModelProtocolManifest
): CapabilityProtocolDescriptor | undefined =>
  manifest?.protocols.find((descriptor) => descriptor.protocol_id === capability.protocol);

/** Match exact schemes plus the registry's parameterized header/query vocabulary. */
export const isProtocolAuthSchemeAllowed = (
  authScheme: string,
  allowedAuthSchemes: readonly string[]
): boolean => {
  const normalized = authScheme.trim();
  if (allowedAuthSchemes.length === 0) return true;
  if (!normalized) return false;
  return allowedAuthSchemes.some((allowed) => {
    if (allowed === normalized) return true;
    if (allowed === 'header_key:<name>') {
      return normalized.startsWith('header_key:') && normalized.slice('header_key:'.length).trim().length > 0;
    }
    if (allowed === 'query_key:<param>') {
      return normalized.startsWith('query_key:') && normalized.slice('query_key:'.length).trim().length > 0;
    }
    return false;
  });
};

export const effectiveBaseUrl = (
  capability: ModelCapabilityDraft,
  _manifest: ModelProtocolManifest | undefined,
  providerBaseUrl: string,
  connections: readonly ProviderConnectionDescriptor[] = []
): string => {
  if (capability.baseUrlOverride.trim()) return capability.baseUrlOverride.trim();
  const connectionRole = capability.connectionRole.trim() || 'default';
  if (connectionRole !== 'default') {
    return connections.find((connection) => connection.role === connectionRole)?.base_url.trim() ?? '';
  }
  return providerBaseUrl.trim();
};

export const endpointDescriptorValue = (
  capability: ModelCapabilityDraft,
  descriptor: CapabilityEndpointDescriptor
): string => {
  switch (descriptor.field) {
    case 'endpoint':
      return capability.endpoint || descriptor.default_value || '';
    case 'poll_endpoint':
      return capability.pollEndpoint || descriptor.default_value || '';
    case 'content_endpoint':
      return capability.contentEndpoint || descriptor.default_value || '';
    case 'realtime_endpoint':
      return capability.realtimeEndpoint || descriptor.default_value || '';
  }
  return '';
};

/**
 * Does this path segment name an API version? Mirrors Rust
 * `url_algebra::is_version_segment`.
 */
const isVersionSegment = (segment: string): boolean =>
  /^v\d[a-z0-9]*$/i.test(segment);

const pathSegments = (path: string): string[] => path.split('/').filter((segment) => segment.length > 0);

const isAbsoluteUrl = (value: string): boolean => {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
};

const rootPathSegments = (baseUrl: string): string[] | undefined => {
  try {
    return pathSegments(new URL(baseUrl.trim()).pathname);
  } catch {
    return undefined;
  }
};

/** Does any path segment of this root name an API version? */
export const rootDeclaresVersion = (baseUrl: string): boolean =>
  (rootPathSegments(baseUrl) ?? []).some(isVersionSegment);

/**
 * Is this base URL shaped the way the protocol's endpoint template expects?
 * Mirrors Rust `url_algebra::root_matches_shape`.
 */
export const rootMatchesShape = (baseUrl: string, shape: EndpointRootShape): boolean =>
  shape === 'versioned_root' ? rootDeclaresVersion(baseUrl) : !rootDeclaresVersion(baseUrl);

/**
 * Join a connection root and an endpoint template, collapsing a duplicated
 * version seam exactly once.
 *
 * A deliberate second implementation of Rust `url_algebra::join_endpoint`: a
 * live per-keystroke preview cannot round-trip to the backend. Both are locked
 * to the shared `url_join_cases.json` fixture so they cannot drift.
 */
export const joinEndpointUrl = (baseUrl: string, endpoint: string): string => {
  const template = endpoint.trim();
  // An absolute endpoint wins verbatim — the escape hatch for a provider whose
  // real path genuinely repeats a version segment.
  if (isAbsoluteUrl(template)) return template;

  const root = baseUrl.trim().replace(/\/+$/, '');
  const tailIndex = template.search(/[?#]/);
  const templatePath = tailIndex >= 0 ? template.slice(0, tailIndex) : template;
  const tail = tailIndex >= 0 ? template.slice(tailIndex) : '';
  const templateSegments = pathSegments(templatePath);
  const rootSegments = rootPathSegments(root) ?? [];

  let drop = 0;
  const max = Math.min(rootSegments.length, templateSegments.length);
  for (let take = max; take >= 1; take -= 1) {
    const rootTail = rootSegments.slice(rootSegments.length - take);
    const matches = rootTail.every(
      (segment, index) => segment.toLowerCase() === templateSegments[index]?.toLowerCase()
    );
    // Only a version seam is de-duplicated: a repeated non-version segment
    // (`/videos` + `/videos/{id}`) is a real path.
    if (matches && rootTail.some(isVersionSegment)) {
      drop = take;
      break;
    }
  }

  const remaining = templateSegments.slice(drop);
  return remaining.length === 0 ? `${root}${tail}` : `${root}/${remaining.join('/')}${tail}`;
};

/**
 * The exact URL a request will hit for one capability endpoint.
 *
 * This value previously appeared nowhere in the UI, which is why a user could
 * pair a base URL ending in `/v1` with a documented path of
 * `/v1/chat/completions` and only discover the doubled segment as a 404.
 */
export const resolvedCapabilityUrl = (
  capability: ModelCapabilityDraft,
  descriptor: CapabilityEndpointDescriptor,
  manifest: ModelProtocolManifest | undefined,
  providerBaseUrl: string,
  connections: readonly ProviderConnectionDescriptor[] = []
): string => {
  const base = effectiveBaseUrl(capability, manifest, providerBaseUrl, connections);
  const template = endpointDescriptorValue(capability, descriptor);
  if (!base.trim() || !template.trim()) return '';
  return joinEndpointUrl(base, template);
};

const urlOrigin = (value: string): string | undefined => {
  try {
    const parsed = new URL(value);
    if (!['http:', 'https:', 'ws:', 'wss:'].includes(parsed.protocol)) return undefined;
    const normalizedProtocol = parsed.protocol === 'ws:' ? 'http:' : parsed.protocol === 'wss:' ? 'https:' : parsed.protocol;
    return `${normalizedProtocol}//${parsed.host}`.toLowerCase();
  } catch {
    return undefined;
  }
};

/** Credentials may leave the provider origin only after explicit consent. */
export const requiresCrossOriginConsent = (
  capability: ModelCapabilityDraft,
  manifest: ModelProtocolManifest | undefined,
  providerBaseUrl: string,
  connections: readonly ProviderConnectionDescriptor[] = []
): boolean => {
  const connectionRole = capability.connectionRole.trim() || 'default';
  const credentialBaseUrl =
    connectionRole === 'default'
      ? providerBaseUrl
      : connections.find((connection) => connection.role === connectionRole)?.base_url ?? '';
  const credentialOrigin = urlOrigin(credentialBaseUrl);
  if (!credentialOrigin) return false;
  const candidates = [
    effectiveBaseUrl(capability, manifest, providerBaseUrl, connections),
    capability.endpoint,
    capability.pollEndpoint,
    capability.contentEndpoint,
    capability.realtimeEndpoint,
  ];
  return candidates.some((candidate) => {
    const origin = urlOrigin(candidate.trim());
    return origin !== undefined && origin !== credentialOrigin;
  });
};

export const parseProviderParams = (
  raw: string
): { ok: true; value: Record<string, unknown> } | { ok: false } => {
  if (!raw.trim()) return { ok: true, value: {} };
  try {
    const parsed = JSON.parse(raw);
    return isPlainObject(parsed) ? { ok: true, value: parsed } : { ok: false };
  } catch {
    return { ok: false };
  }
};

/**
 * Read `provider_params.voice` out of the raw JSON draft.
 *
 * Adapters that require a provider voice (StepFun) reject the request locally
 * when it is absent, so the dedicated voice control reads and writes this same
 * JSON rather than keeping a parallel draft field: two sources would let the
 * textarea and the picker disagree about what is about to be saved.
 *
 * Returns `''` for blank, malformed, or non-string values — none of them are a
 * usable voice id, and showing one as if it were would hide the real problem.
 */
export const providerParamVoice = (raw: string): string => {
  const parsed = parseProviderParams(raw);
  if (!parsed.ok) return '';
  const voice = parsed.value.voice;
  return typeof voice === 'string' ? voice.trim() : '';
};

/**
 * Write `voice` into the raw JSON draft, preserving every other param.
 *
 * A blank voice DELETES the key: persisting `""` would still fail the
 * adapter's non-empty check while looking configured in the UI. Malformed JSON
 * is returned untouched so a typo in the textarea cannot silently discard what
 * the user typed.
 */
export const withProviderParamVoice = (raw: string, voice: string): string => {
  const parsed = parseProviderParams(raw);
  if (!parsed.ok) return raw;
  const trimmed = voice.trim();
  const next = { ...parsed.value };
  if (trimmed) next.voice = trimmed;
  else delete next.voice;
  return Object.keys(next).length > 0 ? JSON.stringify(next, null, 2) : '';
};

export const validateModelDefinition = (
  definition: ModelDefinitionDraft,
  manifests: ModelProtocolManifestMap,
  providerBaseUrl: string,
  existingModelIds: readonly string[] = [],
  loadingTasks: readonly ModelTask[] = [],
  availableConnectionRoles: readonly string[] = [],
  providerAuthScheme = '',
  connectionAuthSchemes: Readonly<Record<string, string>> = {},
  connections: readonly ProviderConnectionDescriptor[] = []
): CapabilityValidationResult => {
  const errors: CapabilityValidationResult['errors'] = [];
  if (!normalizeModelId(definition.model)) errors.push({ code: 'model_required' });
  else if (isDuplicateModelId(definition.model, existingModelIds)) errors.push({ code: 'duplicate_model' });
  if (definition.capabilities.length === 0) errors.push({ code: 'capability_required' });

  for (const capability of definition.capabilities) {
    if (loadingTasks.includes(capability.task)) {
      errors.push({ task: capability.task, code: 'manifest_loading' });
      continue;
    }
    const manifest = manifests[capability.task];
    if (!manifest) {
      errors.push({ task: capability.task, code: 'manifest_unavailable' });
      continue;
    }
    if (!capability.protocol.trim()) {
      errors.push({ task: capability.task, code: 'protocol_required' });
    } else if (!manifest.protocols.some((descriptor) => descriptor.protocol_id === capability.protocol.trim())) {
      errors.push({ task: capability.task, code: 'protocol_not_registered' });
    }
    const descriptor = protocolDescriptorForDraft(capability, manifest);
    const selectedAuthScheme =
      capability.connectionRole.trim() === 'default'
        ? providerAuthScheme
        : connectionAuthSchemes[capability.connectionRole.trim()] ?? '';
    if (
      descriptor &&
      selectedAuthScheme &&
      !isProtocolAuthSchemeAllowed(selectedAuthScheme, descriptor.allowed_auth_schemes)
    ) {
      errors.push({ task: capability.task, code: 'auth_scheme_incompatible' });
    }
    const connectionRole = capability.connectionRole.trim();
    if (!connectionRole) {
      errors.push({ task: capability.task, code: 'connection_role_required' });
    } else if (
      connectionRole !== 'default' &&
      !availableConnectionRoles.includes(connectionRole)
    ) {
      errors.push({ task: capability.task, code: 'connection_missing' });
    }
    const connectionResolvable =
      connectionRole === 'default' || availableConnectionRoles.includes(connectionRole);
    const connectionUrlKnown =
      connectionRole === 'default' || connections.some((connection) => connection.role === connectionRole);
    if (
      descriptor?.transport !== 'sdk' &&
      connectionResolvable &&
      connectionUrlKnown &&
      !effectiveBaseUrl(capability, manifest, providerBaseUrl, connections).trim()
    ) {
      errors.push({ task: capability.task, code: 'base_url_required' });
    }
    if (
      requiresCrossOriginConsent(capability, manifest, providerBaseUrl, connections) &&
      !capability.allowCrossOriginCredentials
    ) {
      errors.push({ task: capability.task, code: 'cross_origin_consent_required' });
    }
    if (!parseProviderParams(capability.providerParamsJson).ok) {
      errors.push({ task: capability.task, code: 'invalid_provider_params' });
    }
  }
  return { valid: errors.length === 0, errors };
};

const optionalTrimmed = (value: string): string | undefined => value.trim() || undefined;

/** Serialize one complete task capability for the canonical full-save request. */
export const capabilityInputFromDraft = (
  capability: ModelCapabilityDraft
): ProviderModelCapabilityInput | undefined => {
  const providerParams = parseProviderParams(capability.providerParamsJson);
  if (!providerParams.ok) return undefined;
  return {
    task: capability.task,
    ...(capability.traits.length > 0 ? { traits: capability.traits } : {}),
    protocol: capability.protocol.trim(),
    connection_role: capability.connectionRole.trim(),
    ...(optionalTrimmed(capability.baseUrlOverride)
      ? { base_url_override: optionalTrimmed(capability.baseUrlOverride) }
      : {}),
    ...(optionalTrimmed(capability.endpoint) ? { endpoint: optionalTrimmed(capability.endpoint) } : {}),
    ...(optionalTrimmed(capability.pollEndpoint)
      ? { poll_endpoint: optionalTrimmed(capability.pollEndpoint) }
      : {}),
    ...(optionalTrimmed(capability.contentEndpoint)
      ? { content_endpoint: optionalTrimmed(capability.contentEndpoint) }
      : {}),
    ...(optionalTrimmed(capability.realtimeEndpoint)
      ? { realtime_endpoint: optionalTrimmed(capability.realtimeEndpoint) }
      : {}),
    ...(capability.allowCrossOriginCredentials ? { allow_cross_origin_credentials: true } : {}),
    ...(Object.keys(providerParams.value).length > 0 ? { provider_params: providerParams.value } : {}),
    ...(capability.contextLimit && capability.contextLimit > 0
      ? { context_limit: capability.contextLimit }
      : {}),
  };
};

/** Strip response-only health/timestamps while preserving the canonical task configuration. */
export const capabilityInputFromResponse = (
  capability: Parameters<typeof capabilityDraftFromResponse>[0]
): ProviderModelCapabilityInput => capabilityInputFromDraft(capabilityDraftFromResponse(capability))!;

export const capabilityInputsFromDefinition = (
  definition: ModelDefinitionDraft
): ProviderModelCapabilityInput[] | undefined => {
  const capabilities = definition.capabilities.map(capabilityInputFromDraft);
  return capabilities.every((capability): capability is ProviderModelCapabilityInput => capability !== undefined)
    ? capabilities
    : undefined;
};
