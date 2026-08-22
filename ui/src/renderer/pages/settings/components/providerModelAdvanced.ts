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
  /**
   * UI-only ownership for protocol-dependent fields. Runtime never sees this
   * value: it exists so an async recommendation may update its own previous
   * value without overwriting a user edit or an already-persisted capability.
   */
  transportSource: 'blank' | 'recommendation' | 'user' | 'persisted';
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
  outputLimit?: number;
}

export type ModelCapabilityDraftPatch = Partial<
  Omit<ModelCapabilityDraft, 'task' | 'transportSource'>
>;

export interface ModelDefinitionDraft {
  model: string;
  capabilities: ModelCapabilityDraft[];
}

export interface CatalogCapabilitySuggestion {
  model: string;
  tasks: ModelTask[];
  traits: ModelTrait[];
  /** Context window the provider's catalog declares, when it declares one. */
  contextLimit?: number;
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
  | 'output_ceiling_required'
  | 'cross_origin_consent_required'
  | 'invalid_provider_params';

export interface CapabilityValidationResult {
  valid: boolean;
  errors: Array<{ task?: ModelTask; code: CapabilityValidationError }>;
}

/**
 * The i18n key naming what a validation code actually asks the user to do.
 *
 * Every code used to be invisible: the capability card rendered a bare count
 * ("待处理 1 项"), the offending control showed only a red border, and the save
 * toast named no field. A `new-api` provider legitimately requires an explicit
 * per-model protocol, so a fully-filled form reported "incomplete" with nothing
 * to act on and the model could not be created at all.
 */
export const capabilityValidationMessageKey = (code: CapabilityValidationError): string =>
  `settings.capabilityError.${code}`;

/**
 * One sentence naming every blocker, task-scoped codes prefixed by their task.
 *
 * Takes a translate callback so this module stays free of the i18n runtime and
 * remains unit-testable. Returns `''` when there is nothing to report, letting
 * the caller keep its generic fallback for that case.
 */
export const describeValidationErrors = (
  errors: CapabilityValidationResult['errors'],
  translate: (key: string, fallback: string) => string
): string =>
  [
    ...new Set(
      errors.map((error) => {
        const message = translate(capabilityValidationMessageKey(error.code), error.code);
        return error.task
          ? `${translate(`settings.modelTask.${error.task}`, error.task)} · ${message}`
          : message;
      })
    ),
  ].join(' · ');

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
  transportSource: 'blank',
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
  outputLimit: undefined,
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
  output_limit?: number;
}): ModelCapabilityDraft => ({
  task: capability.task,
  traits: capability.traits ?? [],
  transportSource: 'persisted',
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
  outputLimit: capability.output_limit,
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
 * Adopt a catalog entry's model id, enriching only the task it was chosen for.
 *
 * Non-destructive by contract. This used to return a single fresh capability,
 * which silently discarded every other declared task, plus that task's own
 * protocol/endpoint work, the moment a user clicked a suggestion. The catalog is
 * advisory, so it may never overwrite configuration the user already entered.
 *
 * Traits and the context window are the fields it owns, and only when the entry
 * actually declares this task: an entry that says nothing about the task says
 * nothing about its capabilities either. A window the user already chose wins —
 * they may be correcting the provider, which is the whole point of the field.
 */
export const applyCatalogSuggestionForTask = (
  definition: ModelDefinitionDraft,
  suggestion: CatalogCapabilitySuggestion,
  task: ModelTask
): ModelDefinitionDraft => {
  const declaresTask = suggestion.tasks.includes(task);
  const traits = declaresTask
    ? MODEL_TRAIT_ORDER.filter(
        (trait) => suggestion.traits.includes(trait) && CATALOG_TRAITS_BY_TASK[task].includes(trait)
      )
    : [];
  const declaredWindow =
    declaresTask && suggestion.contextLimit && suggestion.contextLimit > 0
      ? suggestion.contextLimit
      : undefined;
  const known = definition.capabilities.some((capability) => capability.task === task);
  return {
    model: suggestion.model,
    capabilities: known
      ? definition.capabilities.map((capability) =>
          capability.task === task && declaresTask
            ? {
                ...capability,
                traits,
                contextLimit: capability.contextLimit ?? declaredWindow,
              }
            : capability
        )
      : [
          ...definition.capabilities,
          { ...emptyCapabilityDraft(task), traits, contextLimit: declaredWindow },
        ],
  };
};

/**
 * Apply backend recommendations only to blank or recommendation-owned
 * transport. User-edited and persisted transport remain authoritative.
 */
export const reconcileCapabilityRecommendations = (
  capabilities: readonly ModelCapabilityDraft[],
  manifests: ModelProtocolManifestMap
): ModelCapabilityDraft[] =>
  capabilities.map((capability) => {
    const manifest = manifests[capability.task];
    // Missing data means the request is loading or failed. Only a resolved
    // manifest with an explicit null recommendation may withdraw an automatic
    // value; transient transport state must not mutate the user's draft.
    if (!manifest) return capability;
    const recommendation = manifest.recommendation;
    if (capability.transportSource === 'user' || capability.transportSource === 'persisted') {
      return capability;
    }

    if (!recommendation) {
      return capability.transportSource === 'recommendation'
        ? resetCapabilityTransport(capability, 'blank')
        : capability;
    }

    const protocol = recommendation.protocol_id.trim();
    const connectionRole = recommendation.connection_role || 'default';
    const baseUrlOverride =
      connectionRole === 'default' &&
      recommendation.base_url_override_required &&
      recommendation.default_base_url
        ? recommendation.default_base_url
        : '';
    const protocolChanged = capability.protocol.trim() !== protocol;
    const base = protocolChanged
      ? resetCapabilityTransport(capability, 'recommendation')
      : capability;
    return base.protocol === protocol &&
      base.connectionRole === connectionRole &&
      base.baseUrlOverride === baseUrlOverride &&
      base.transportSource === 'recommendation'
      ? base
      : {
          ...base,
          protocol,
          connectionRole,
          baseUrlOverride,
          transportSource: 'recommendation',
        };
  });

const resetCapabilityTransport = (
  capability: ModelCapabilityDraft,
  transportSource: ModelCapabilityDraft['transportSource']
): ModelCapabilityDraft => ({
  ...capability,
  transportSource,
  protocol: '',
  connectionRole: 'default',
  baseUrlOverride: '',
  endpoint: '',
  pollEndpoint: '',
  contentEndpoint: '',
  realtimeEndpoint: '',
  allowCrossOriginCredentials: false,
  providerParamsJson: '',
});

const TRANSPORT_DRAFT_FIELDS = new Set<keyof ModelCapabilityDraft>([
  'protocol',
  'connectionRole',
  'baseUrlOverride',
  'endpoint',
  'pollEndpoint',
  'contentEndpoint',
  'realtimeEndpoint',
  'allowCrossOriginCredentials',
  'providerParamsJson',
]);

/** Apply an editor patch and transfer recommendation-owned transport to the user. */
export const patchCapabilityDraft = (
  capability: ModelCapabilityDraft,
  patch: ModelCapabilityDraftPatch
): ModelCapabilityDraft => ({
  ...capability,
  ...patch,
  task: capability.task,
  ...(Object.keys(patch).some((key) =>
    TRANSPORT_DRAFT_FIELDS.has(key as keyof ModelCapabilityDraft)
  )
    ? { transportSource: 'user' as const }
    : {}),
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
  if (normalizedProtocol === capability.protocol.trim()) {
    return capability.transportSource === 'recommendation' || capability.transportSource === 'blank'
      ? { ...capability, transportSource: 'user' }
      : capability;
  }
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
    ...resetCapabilityTransport(capability, 'user'),
    protocol: normalizedProtocol,
    connectionRole,
    baseUrlOverride,
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

/** Read the explicit Responses round-chaining opt-in from provider params. */
export const providerParamChainRounds = (raw: string): boolean => {
  const parsed = parseProviderParams(raw);
  return parsed.ok && parsed.value.chain_rounds === true;
};

/**
 * Toggle Responses round chaining in the one canonical provider-params JSON.
 *
 * Disabled is represented by an absent key, not `false`: omission keeps the
 * protocol's privacy-preserving default authoritative. Malformed JSON is
 * returned byte-for-byte so this structured control can never erase a user's
 * unfinished raw edit.
 */
export const withProviderParamChainRounds = (raw: string, enabled: boolean): string => {
  const parsed = parseProviderParams(raw);
  if (!parsed.ok) return raw;
  const next = { ...parsed.value };
  if (enabled) next.chain_rounds = true;
  else delete next.chain_rounds;
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
    if (
      descriptor?.requires_output_ceiling &&
      !(
        typeof capability.outputLimit === 'number' &&
        Number.isFinite(capability.outputLimit) &&
        capability.outputLimit > 0
      )
    ) {
      errors.push({ task: capability.task, code: 'output_ceiling_required' });
    }
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
    ...(capability.outputLimit && capability.outputLimit > 0
      ? { output_limit: capability.outputLimit }
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
