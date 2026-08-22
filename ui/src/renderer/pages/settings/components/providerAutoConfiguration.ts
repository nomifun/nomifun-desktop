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
  currentProtocol: string
): ProtocolDescriptor[] => {
  const preferredIds = [
    currentProtocol.trim(),
    manifest.recommendation?.protocol_id ?? '',
    manifest.platform === 'new-api' && task === 'chat' ? 'openai.chat_text' : '',
  ].filter(Boolean);
  const byId = new Map(manifest.protocols.map((descriptor) => [descriptor.protocol_id, descriptor]));
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
  useStoredAuth: boolean
): ProviderAutoConfigurationTarget[] =>
  definition.capabilities.flatMap((capability) => {
    if (capability.transportSource === 'user' || capability.transportSource === 'persisted') {
      return [];
    }
    const manifest = manifests[capability.task];
    if (!manifest) return [];
    const candidates = orderedDescriptors(
      manifest,
      capability.task,
      capability.protocol
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
