/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProbeProviderConnectionResponse } from '@/common/protocolBindings/ProbeProviderConnectionResponse';
import type { ProviderReachability } from '@/common/protocolBindings/ProviderReachability';
import type { ProtocolDescriptor } from '@/common/types/provider/modelProtocolManifest';
import {
  isProtocolAuthSchemeAllowed,
  type ModelDefinitionDraft,
  type ModelProtocolManifest,
  type ModelProtocolManifestMap,
} from './providerModelAdvanced';

export const DEFAULT_REQUIRED_OUTPUT_LIMIT = 4_096;

export type ProviderCompatibilityMode = 'auto' | 'openai' | 'anthropic';

const OPENAI_PROTOCOL_BY_TASK: Readonly<Partial<Record<ModelTask, string>>> = {
  chat: 'openai.chat_text',
  image_generation: 'openai.images',
  image_edit: 'openai.images',
  video_generation: 'openai.videos',
  speech_synthesis: 'openai.audio_speech',
  speech_recognition: 'openai.audio_transcriptions',
  embedding: 'openai.embeddings',
  rerank: 'generic.rerank',
};

const ANTHROPIC_PROTOCOL_BY_TASK: Readonly<Partial<Record<ModelTask, string>>> = {
  chat: 'anthropic.messages',
};

const EMPTY_PROTOCOL_BY_TASK: Readonly<Partial<Record<ModelTask, string>>> = {};

export type ProviderAutoConfigurationConfidence =
  | 'verified'
  | 'endpoint_confirmed'
  | 'fallback';

export interface ProviderAutoConfigurationCandidate {
  descriptor: ProtocolDescriptor;
  authScheme: string;
}

export interface ProviderAutoConfigurationTarget {
  task: ModelTask;
  candidates: ProviderAutoConfigurationCandidate[];
}

export interface ProviderAutoConfigurationProbeAttempt {
  candidate: ProviderAutoConfigurationCandidate;
  response?: ProbeProviderConnectionResponse;
}

export interface ProviderAutoConfigurationDetection {
  task: ModelTask;
  protocol: string;
  authScheme: string;
  confidence: ProviderAutoConfigurationConfidence;
  suggestedBaseUrl?: string;
  outputLimit?: number;
}

export const isAutoConfigurationPlatform = (platform: string): boolean => {
  const normalized = platform.trim().toLowerCase();
  return normalized === 'custom' || normalized === 'new-api';
};

export const providerCompatibilityAuthScheme = (
  mode: ProviderCompatibilityMode
): string | undefined => {
  if (mode === 'openai') return 'bearer';
  if (mode === 'anthropic') return 'header_key:x-api-key';
  return undefined;
};

export const providerCompatibilityProtocolPreferences = (
  mode: ProviderCompatibilityMode
): Readonly<Partial<Record<ModelTask, string>>> => {
  if (mode === 'openai') return OPENAI_PROTOCOL_BY_TASK;
  if (mode === 'anthropic') return ANTHROPIC_PROTOCOL_BY_TASK;
  return EMPTY_PROTOCOL_BY_TASK;
};

export const providerCompatibilityProtocolForTask = (
  mode: ProviderCompatibilityMode,
  task: ModelTask
): string | undefined => providerCompatibilityProtocolPreferences(mode)[task];

const isVersionSegment = (segment: string): boolean => /^v\d[a-z0-9]*$/i.test(segment);

/** Keep the provider root aligned with the selected wire format's URL shape. */
export const normalizeProviderBaseUrlForCompatibilityMode = (
  baseUrl: string,
  mode: ProviderCompatibilityMode
): string => {
  const normalized = baseUrl.trim();
  if (!normalized || mode === 'auto') return normalized;
  try {
    const url = new URL(normalized);
    const segments = url.pathname.split('/').filter(Boolean);
    if (mode === 'openai' && !segments.some(isVersionSegment)) {
      segments.push('v1');
    } else if (mode === 'anthropic' && segments.length > 0 && isVersionSegment(segments.at(-1)!)) {
      segments.pop();
    }
    url.pathname = segments.length > 0 ? `/${segments.join('/')}` : '';
    return `${url.origin}${url.pathname}${url.search}${url.hash}`;
  } catch {
    return normalized;
  }
};

const concreteAuthScheme = (scheme: string, currentScheme: string): string | undefined => {
  const normalized = scheme.trim();
  if (!normalized) return undefined;
  if (normalized === 'header_key:<name>') {
    return currentScheme.startsWith('header_key:')
      ? currentScheme
      : 'header_key:x-api-key';
  }
  if (normalized === 'query_key:<param>') {
    return currentScheme.startsWith('query_key:')
      ? currentScheme
      : 'query_key:key';
  }
  return normalized;
};

/**
 * Use the user's scheme when the adapter accepts it. Otherwise select the
 * adapter's first concrete scheme so the same API key can be probed with the
 * header/query shape the endpoint actually expects.
 */
export const selectProbeAuthScheme = (
  descriptor: ProtocolDescriptor,
  currentScheme: string
): string | undefined => {
  const normalizedCurrent = currentScheme.trim();
  if (
    normalizedCurrent &&
    isProtocolAuthSchemeAllowed(normalizedCurrent, descriptor.allowed_auth_schemes)
  ) {
    return normalizedCurrent;
  }
  for (const allowed of descriptor.allowed_auth_schemes) {
    const concrete = concreteAuthScheme(allowed, normalizedCurrent);
    if (concrete) return concrete;
  }
  return undefined;
};

const protocolCanBeProbed = (descriptor: ProtocolDescriptor, task: ModelTask): boolean =>
  descriptor.transport !== 'sdk' &&
  descriptor.supported_tasks.includes(task) &&
  descriptor.endpoints.some(
    (endpoint) => endpoint.task === task && endpoint.field === 'endpoint'
  );

const orderedDescriptors = (
  manifest: ModelProtocolManifest,
  task: ModelTask,
  currentProtocol: string,
  preferredProtocol?: string
): ProtocolDescriptor[] => {
  const byId = new Map(manifest.protocols.map((descriptor) => [descriptor.protocol_id, descriptor]));
  if (preferredProtocol) {
    const descriptor = byId.get(preferredProtocol);
    return descriptor && protocolCanBeProbed(descriptor, task) ? [descriptor] : [];
  }
  const preferredIds = [
    currentProtocol.trim(),
    manifest.recommendation?.protocol_id ?? '',
    manifest.platform === 'new-api' && task === 'chat' ? 'openai.chat_text' : '',
  ].filter(Boolean);
  const ordered: ProtocolDescriptor[] = [];
  for (const protocolId of preferredIds) {
    const descriptor = byId.get(protocolId);
    if (descriptor && protocolCanBeProbed(descriptor, task)) ordered.push(descriptor);
  }
  for (const descriptor of manifest.protocols) {
    if (
      protocolCanBeProbed(descriptor, task) &&
      !ordered.some((candidate) => candidate.protocol_id === descriptor.protocol_id)
    ) {
      ordered.push(descriptor);
    }
  }
  return ordered;
};

export const buildProviderAutoConfigurationTargets = (
  definition: ModelDefinitionDraft,
  manifests: ModelProtocolManifestMap,
  currentAuthScheme: string,
  useStoredAuth: boolean,
  protocolPreferences: Readonly<Partial<Record<ModelTask, string>>> = {}
): ProviderAutoConfigurationTarget[] =>
  definition.capabilities.flatMap((capability) => {
    const preferredProtocol = protocolPreferences[capability.task];
    if (
      (capability.transportSource === 'user' ||
        capability.transportSource === 'persisted') &&
      !preferredProtocol
    ) {
      return [];
    }
    const manifest = manifests[capability.task];
    if (!manifest) return [];
    const candidates = orderedDescriptors(
      manifest,
      capability.task,
      capability.protocol,
      preferredProtocol
    ).flatMap((descriptor) => {
      const authScheme = useStoredAuth
        ? currentAuthScheme.trim()
        : selectProbeAuthScheme(descriptor, currentAuthScheme);
      if (!authScheme) return [];
      if (
        useStoredAuth &&
        !isProtocolAuthSchemeAllowed(authScheme, descriptor.allowed_auth_schemes)
      ) {
        return [];
      }
      return [{ descriptor, authScheme }];
    });
    return candidates.length > 0 ? [{ task: capability.task, candidates }] : [];
  });

const effectiveProbeOutcome = (
  response: ProbeProviderConnectionResponse
): { reachability: ProviderReachability; suggestedBaseUrl?: string } => {
  const suggestedBaseUrl = response.suggested_base_url?.trim();
  if (suggestedBaseUrl) {
    const suggested = response.candidates?.find(
      (candidate) => candidate.base_url.trim() === suggestedBaseUrl
    );
    if (suggested && suggested.reachability !== 'unreachable') {
      return {
        reachability: suggested.reachability,
        suggestedBaseUrl,
      };
    }
  }
  return { reachability: response.reachability };
};

const reachabilityRank = (reachability: ProviderReachability): number => {
  switch (reachability) {
    case 'reachable':
      return 2;
    case 'credentials_rejected':
      return 1;
    case 'unreachable':
      return 0;
  }
};

/**
 * Probe order is deterministic and is the tie-breaker. A parsed endpoint wins
 * over a credential rejection; a credential rejection still proves the route
 * exists and is therefore better than leaving the protocol blank.
 */
export const selectProviderAutoConfiguration = (
  target: ProviderAutoConfigurationTarget,
  attempts: readonly ProviderAutoConfigurationProbeAttempt[]
): ProviderAutoConfigurationDetection | undefined => {
  let best:
    | {
        attempt: ProviderAutoConfigurationProbeAttempt;
        reachability: ProviderReachability;
        suggestedBaseUrl?: string;
        rank: number;
      }
    | undefined;
  for (const attempt of attempts) {
    if (!attempt.response) continue;
    const outcome = effectiveProbeOutcome(attempt.response);
    const rank = reachabilityRank(outcome.reachability);
    if (rank > 0 && (!best || rank > best.rank)) {
      best = { attempt, ...outcome, rank };
    }
  }

  const selected = best?.attempt.candidate ?? target.candidates[0];
  if (!selected) return undefined;
  return {
    task: target.task,
    protocol: selected.descriptor.protocol_id,
    authScheme: selected.authScheme,
    confidence:
      best?.reachability === 'reachable'
        ? 'verified'
        : best?.reachability === 'credentials_rejected'
          ? 'endpoint_confirmed'
          : 'fallback',
    ...(best?.suggestedBaseUrl
      ? { suggestedBaseUrl: best.suggestedBaseUrl }
      : {}),
    ...(selected.descriptor.requires_output_ceiling
      ? { outputLimit: DEFAULT_REQUIRED_OUTPUT_LIMIT }
      : {}),
  };
};

const sameDefinition = (
  left: ModelDefinitionDraft,
  right: ModelDefinitionDraft
): boolean => JSON.stringify(left) === JSON.stringify(right);

/**
 * Apply a one-click compatibility preset. A direct mode switch is allowed to
 * replace manual transport fields; later-added tasks only receive the preset
 * while they remain blank or recommendation-owned.
 */
export const applyProviderCompatibilityMode = (
  definition: ModelDefinitionDraft,
  mode: ProviderCompatibilityMode,
  force = false
): ModelDefinitionDraft => {
  if (mode === 'auto') {
    if (!force) return definition;
    const knownProtocols = new Set([
      ...Object.values(OPENAI_PROTOCOL_BY_TASK),
      ...Object.values(ANTHROPIC_PROTOCOL_BY_TASK),
    ]);
    const next: ModelDefinitionDraft = {
      ...definition,
      capabilities: definition.capabilities.map((capability) => {
        if (
          capability.transportSource !== 'user' ||
          !knownProtocols.has(capability.protocol.trim())
        ) {
          return capability;
        }
        return {
          ...capability,
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
          outputLimit: undefined,
        };
      }),
    };
    return sameDefinition(definition, next) ? definition : next;
  }
  const protocols = providerCompatibilityProtocolPreferences(mode);
  const next: ModelDefinitionDraft = {
    ...definition,
    capabilities: definition.capabilities.map((capability) => {
      const protocol = protocols[capability.task];
      if (!protocol) {
        if (
          mode === 'anthropic' &&
          capability.transportSource === 'recommendation'
        ) {
          return {
            ...capability,
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
            outputLimit: undefined,
          };
        }
        return capability;
      }
      if (
        !force &&
        (capability.transportSource === 'user' ||
          capability.transportSource === 'persisted')
      ) {
        return capability;
      }
      const resetTransport = force || capability.protocol.trim() !== protocol;
      return {
        ...capability,
        // An explicit mode is a user choice. Keep it out of the manifest's
        // automatic recommendation loop, otherwise Custom's OpenAI fallback
        // would immediately overwrite a selected Claude mode.
        transportSource: 'user',
        protocol,
        connectionRole: 'default',
        ...(resetTransport
          ? {
              baseUrlOverride: '',
              endpoint: '',
              pollEndpoint: '',
              contentEndpoint: '',
              realtimeEndpoint: '',
              allowCrossOriginCredentials: false,
              providerParamsJson: '',
            }
          : {}),
        outputLimit:
          protocol === 'anthropic.messages'
            ? resetTransport
              ? DEFAULT_REQUIRED_OUTPUT_LIMIT
              : capability.outputLimit ?? DEFAULT_REQUIRED_OUTPUT_LIMIT
            : resetTransport
              ? undefined
              : capability.outputLimit,
      };
    }),
  };
  return sameDefinition(definition, next) ? definition : next;
};

/**
 * Detection owns only untouched/recommendation-owned transport. A user edit or
 * persisted capability remains authoritative.
 */
export const applyProviderAutoConfiguration = (
  definition: ModelDefinitionDraft,
  detections: readonly ProviderAutoConfigurationDetection[]
): ModelDefinitionDraft => {
  const byTask = new Map(detections.map((detection) => [detection.task, detection]));
  const next: ModelDefinitionDraft = {
    ...definition,
    capabilities: definition.capabilities.map((capability) => {
      const detection = byTask.get(capability.task);
      if (
        !detection ||
        capability.transportSource === 'user' ||
        capability.transportSource === 'persisted'
      ) {
        return capability;
      }
      const protocolChanged = capability.protocol.trim() !== detection.protocol;
      return {
        ...capability,
        transportSource: 'recommendation',
        protocol: detection.protocol,
        connectionRole: 'default',
        ...(protocolChanged
          ? {
              baseUrlOverride: '',
              endpoint: '',
              pollEndpoint: '',
              contentEndpoint: '',
              realtimeEndpoint: '',
              allowCrossOriginCredentials: false,
              providerParamsJson: '',
            }
          : {}),
        ...(capability.outputLimit === undefined && detection.outputLimit !== undefined
          ? { outputLimit: detection.outputLimit }
          : {}),
      };
    }),
  };
  return sameDefinition(definition, next) ? definition : next;
};
