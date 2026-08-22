/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderId } from '@/common/types/ids';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import { useEffect, useMemo, useRef, useState } from 'react';
import useSWR from 'swr';
import {
  buildProviderAutoConfigurationTargets,
  isAutoConfigurationPlatform,
  selectProviderAutoConfiguration,
  type ProviderAutoConfigurationDetection,
  type ProviderAutoConfigurationProbeAttempt,
} from './providerAutoConfiguration';
import type {
  ModelDefinitionDraft,
  ModelProtocolManifestMap,
} from './providerModelAdvanced';

export interface ProviderAutoConfigurationBatch {
  requestBaseUrl: string;
  detections: ProviderAutoConfigurationDetection[];
}

interface UseProviderAutoConfigurationOptions {
  enabled?: boolean;
  platform: string;
  baseUrl: string;
  authScheme: string;
  definition: ModelDefinitionDraft;
  manifests: ModelProtocolManifestMap;
  loadingTasks?: readonly ModelTask[];
  providerId?: ProviderId;
  credentials?: ProviderCredentials;
}

interface ProviderAutoConfigurationRequest {
  id: string;
  platform: string;
  baseUrl: string;
  model: string;
  targets: ReturnType<typeof buildProviderAutoConfigurationTargets>;
  providerId?: ProviderId;
  credentials?: ProviderCredentials;
}

const PROBE_DEBOUNCE_MS = 500;
const EMPTY_TASKS: readonly ModelTask[] = [];

/**
 * Detect protocol/auth/root for untouched Custom and New API task drafts.
 * Credentials never enter SWR's global cache key; only a local revision does.
 */
export const useProviderAutoConfiguration = ({
  enabled = true,
  platform,
  baseUrl,
  authScheme,
  definition,
  manifests,
  loadingTasks = EMPTY_TASKS,
  providerId,
  credentials,
}: UseProviderAutoConfigurationOptions) => {
  const credentialState = useRef({ snapshot: '', revision: 0 });
  const credentialSnapshot =
    credentials === undefined ? 'missing' : JSON.stringify(credentials);
  if (credentialState.current.snapshot !== credentialSnapshot) {
    credentialState.current = {
      snapshot: credentialSnapshot,
      revision: credentialState.current.revision + 1,
    };
  }

  const useStoredAuth = providerId !== undefined;
  const targets = useMemo(
    () =>
      buildProviderAutoConfigurationTargets(
        definition,
        manifests,
        authScheme,
        useStoredAuth
      ).filter((target) => !loadingTasks.includes(target.task)),
    [authScheme, definition.capabilities, loadingTasks, manifests, useStoredAuth]
  );
  const targetKey = JSON.stringify(
    targets.map((target) => [
      target.task,
      target.candidates.map((candidate) => [
        candidate.descriptor.protocol_id,
        candidate.authScheme,
      ]),
    ])
  );
  const canProbe =
    enabled &&
    isAutoConfigurationPlatform(platform) &&
    Boolean(baseUrl.trim()) &&
    targets.length > 0 &&
    (providerId !== undefined || credentials !== undefined);
  const liveRequest = useMemo<ProviderAutoConfigurationRequest | undefined>(() => {
    if (!canProbe) return undefined;
    const normalizedBaseUrl = baseUrl.trim();
    const normalizedModel = definition.model.trim();
    const id = JSON.stringify([
      providerId ?? 'anonymous',
      platform,
      normalizedBaseUrl,
      authScheme.trim(),
      normalizedModel,
      credentialState.current.revision,
      targetKey,
    ]);
    return {
      id,
      platform,
      baseUrl: normalizedBaseUrl,
      model: normalizedModel,
      targets,
      ...(providerId === undefined ? {} : { providerId }),
      ...(credentials === undefined ? {} : { credentials }),
    };
  }, [
    authScheme,
    baseUrl,
    canProbe,
    credentials,
    definition.model,
    platform,
    providerId,
    targetKey,
    targets,
  ]);
  const [debouncedRequest, setDebouncedRequest] =
    useState<ProviderAutoConfigurationRequest>();
  const liveRequestRef = useRef(liveRequest);
  liveRequestRef.current = liveRequest;
  useEffect(() => {
    if (!liveRequest) {
      setDebouncedRequest(undefined);
      return;
    }
    const requestId = liveRequest.id;
    const timeout = globalThis.setTimeout(
      () => {
        const current = liveRequestRef.current;
        if (current?.id === requestId) setDebouncedRequest(current);
      },
      PROBE_DEBOUNCE_MS
    );
    return () => globalThis.clearTimeout(timeout);
  }, [liveRequest?.id]);
  const request =
    liveRequest && debouncedRequest?.id === liveRequest.id
      ? debouncedRequest
      : undefined;
  const swrKey = request ? ['provider-auto-configuration', request.id] : null;

  const state = useSWR<ProviderAutoConfigurationBatch>(
    swrKey,
    async () => {
      if (!request) return { requestBaseUrl: '', detections: [] };
      const requestBaseUrl = request.baseUrl;
      const detections = await Promise.all(
        request.targets.map(async (target) => {
          const attempts = await Promise.all(
            target.candidates.map(
              async (candidate): Promise<ProviderAutoConfigurationProbeAttempt> => {
                try {
                  const response =
                    request.providerId !== undefined
                      ? await ipcBridge.mode.probeProviderConnection.invoke({
                          provider_id: request.providerId,
                          protocol: candidate.descriptor.protocol_id,
                          task: target.task,
                          ...(request.model ? { model: request.model } : {}),
                          probe_candidates: true,
                        })
                      : await ipcBridge.mode.probeConnection.invoke({
                          platform: request.platform,
                          base_url: requestBaseUrl,
                          auth_scheme: candidate.authScheme,
                          credentials: request.credentials ?? {},
                          protocol: candidate.descriptor.protocol_id,
                          task: target.task,
                          ...(request.model ? { model: request.model } : {}),
                          probe_candidates: true,
                        });
                  return { candidate, response };
                } catch {
                  return { candidate };
                }
              }
            )
          );
          return selectProviderAutoConfiguration(target, attempts);
        })
      );
      return {
        requestBaseUrl,
        detections: detections.filter(
          (detection): detection is ProviderAutoConfigurationDetection =>
            detection !== undefined
        ),
      };
    },
    {
      keepPreviousData: false,
      revalidateOnFocus: false,
      shouldRetryOnError: false,
    }
  );
  return {
    ...state,
    isLoading: state.isLoading || (canProbe && request === undefined),
  };
};

export default useProviderAutoConfiguration;
