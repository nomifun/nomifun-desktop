/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderId } from '@/common/types/ids';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  buildProviderAutoConfigurationTargets, isAutoConfigurationPlatform, selectProbeAuthScheme,
  selectProviderAutoConfiguration, type ProviderAutoConfigurationDetection,
  type ProviderAutoConfigurationProbeAttempt, type ProviderAutoConfigurationTarget,
} from './providerAutoConfiguration';
import type { ModelDefinitionDraft, ModelProtocolManifestMap } from './providerModelAdvanced';

export interface ProviderAutoConfigurationBatch {
  requestBaseUrl: string;
  detections: ProviderAutoConfigurationDetection[];
  candidates: ProviderAutoConfigurationDetection[];
  staleTasks?: ModelTask[];
}
interface UseProviderAutoConfigurationOptions {
  enabled?: boolean;
  platform: string;
  baseUrl: string;
  authScheme: string;
  definition: ModelDefinitionDraft;
  manifests: ModelProtocolManifestMap;
  loadingTasks?: readonly ModelTask[];
  protocolPreferences?: Readonly<Partial<Record<ModelTask, string>>>;
  providerId?: ProviderId;
  credentials?: ProviderCredentials;
}
interface InterfaceRequest {
  platform: string;
  baseUrl: string;
  authScheme: string;
  credentialRevision: number;
  providerId?: ProviderId;
  credentials?: ProviderCredentials;
}
interface CompletedTarget {
  requestBaseUrl: string;
  target: ProviderAutoConfigurationTarget;
  attempts: ProviderAutoConfigurationProbeAttempt[];
  validKeys: Set<string>;
}
interface ProbeJob {
  key: string;
  controller: AbortController;
  timer?: ReturnType<typeof globalThis.setTimeout>;
  dispatched: boolean;
  manual: boolean;
}
const PROBE_DEBOUNCE_MS = 500;
const EMPTY_TASKS: readonly ModelTask[] = [];
const EMPTY_PROTOCOL_PREFERENCES: Readonly<Partial<Record<ModelTask, string>>> = {};
const targetKey = (target: ProviderAutoConfigurationTarget): string => JSON.stringify([
  target.task, target.candidates.map(candidate => [candidate.descriptor.protocol_id, candidate.authScheme]).sort(),
]);
const probeKey = (request: InterfaceRequest, target: ProviderAutoConfigurationTarget,
  baseUrl = request.baseUrl, authScheme = request.authScheme): string => JSON.stringify([
  request.providerId ?? 'anonymous', request.platform, baseUrl, authScheme, request.credentialRevision, targetKey(target),
]);
const orderedAttempts = (target: ProviderAutoConfigurationTarget, completed: CompletedTarget) => target.candidates.flatMap(candidate => {
  const attempt = completed.attempts.find(entry => entry.candidate.descriptor.protocol_id === candidate.descriptor.protocol_id);
  return attempt ? [attempt] : [];
});
const stopJob = (job: ProbeJob) => { globalThis.clearTimeout(job.timer); job.controller.abort(); };

/** Each task receives one automatic interface attempt per form; subsequent checks require a click. */
const useProviderAutoConfiguration = ({
  enabled = true, platform, baseUrl, authScheme, definition, manifests,
  loadingTasks = EMPTY_TASKS, protocolPreferences = EMPTY_PROTOCOL_PREFERENCES, providerId, credentials,
}: UseProviderAutoConfigurationOptions) => {
  const credentialState = useRef({ snapshot: '', revision: 0 });
  const snapshot = credentials === undefined ? 'missing' : JSON.stringify(credentials);
  if (credentialState.current.snapshot !== snapshot) {
    credentialState.current = { snapshot, revision: credentialState.current.revision + 1 };
  }
  const request: InterfaceRequest = { platform, baseUrl: baseUrl.trim(), authScheme: authScheme.trim(),
    credentialRevision: credentialState.current.revision, providerId, credentials };
  const scope = JSON.stringify([providerId, platform, request.baseUrl, request.authScheme, request.credentialRevision, protocolPreferences]);
  const remembered = useRef({ scope, targets: new Map<ModelTask, ProviderAutoConfigurationTarget>() });
  if (remembered.current.scope !== scope) remembered.current = { scope, targets: new Map() };
  const completed = useRef(new Map<ModelTask, CompletedTarget>());
  const attempted = useRef(new Set<ModelTask>());
  const jobs = useRef(new Map<ModelTask, ProbeJob>());
  const [revision, setRevision] = useState(0);
  const [manualRevision, setManualRevision] = useState(0);
  const handledManualRevision = useRef(0);
  const targets = useMemo(() => {
    // Previously checked user choices remain visible. A manual recheck can
    // also inspect a configured task without granting another automatic run.
    const available = buildProviderAutoConfigurationTargets({ ...definition,
      capabilities: definition.capabilities.map(capability => ({ ...capability, transportSource: 'recommendation' })),
    }, manifests, authScheme, providerId !== undefined, protocolPreferences);
    return definition.capabilities.flatMap(capability => {
      const target = available.find(candidate => candidate.task === capability.task);
      if (target && !loadingTasks.includes(capability.task)) {
        remembered.current.targets.set(capability.task, target);
        return [target];
      }
      // A model ID can refresh the manifest while the model-independent
      // endpoint check is pending. Keep that same connection's check alive.
      const previous = remembered.current.targets.get(capability.task);
      return previous && (loadingTasks.includes(capability.task) || !manifests[capability.task]) ? [previous] : [];
    });
  }, [authScheme, definition.capabilities, manifests, loadingTasks, providerId, protocolPreferences, scope, enabled]);
  const automaticTasks = definition.capabilities.filter(capability =>
    (capability.transportSource !== 'user' && capability.transportSource !== 'persisted') || protocolPreferences[capability.task]
  ).map(capability => capability.task);
  const ready = enabled && isAutoConfigurationPlatform(platform) && Boolean(request.baseUrl) &&
    (providerId !== undefined || credentials !== undefined);
  const current = useRef({ request, targets, ready, automaticTasks });
  current.current = { request, targets, ready, automaticTasks };
  const schedulingKey = JSON.stringify([scope, targets.map(target => [targetKey(target), automaticTasks.includes(target.task)]).sort()]);
  const selectionKey = JSON.stringify(targets.map(target => [target.task, target.candidates.map(candidate => candidate.descriptor.protocol_id)]));
  const selectedTasks = definition.capabilities.map(capability => capability.task);
  const selectedTaskKey = JSON.stringify(selectedTasks);

  const data = useMemo<ProviderAutoConfigurationBatch | undefined>(() => {
    const live = current.current;
    const detections: ProviderAutoConfigurationDetection[] = [];
    const candidates: ProviderAutoConfigurationDetection[] = [];
    const staleTasks: ModelTask[] = [];
    let requestBaseUrl: string | undefined;
    for (const task of selectedTasks) {
      const record = completed.current.get(task);
      const target = live.targets.find(candidate => candidate.task === task);
      const key = target ? probeKey(live.request, target) : undefined;
      const pending = key !== undefined && jobs.current.get(task)?.key === key;
      const valid = live.ready && key !== undefined && record?.validKeys.has(key);
      if (!valid && (record || (attempted.current.has(task) && !pending))) staleTasks.push(task);
      if (!record) continue;
      requestBaseUrl ??= record.requestBaseUrl;
      for (const attempt of record.attempts) {
        const candidate = selectProviderAutoConfiguration({ task, candidates: [attempt.candidate] }, [attempt]);
        if (candidate) candidates.push({ ...candidate, requestBaseUrl: record.requestBaseUrl });
      }
      if (valid && target) {
        const detection = selectProviderAutoConfiguration(target, orderedAttempts(target, record));
        if (detection) detections.push({ ...detection, requestBaseUrl: record.requestBaseUrl });
      }
    }
    return candidates.length > 0 || staleTasks.length > 0
      ? { requestBaseUrl: requestBaseUrl ?? live.request.baseUrl, detections, candidates, staleTasks }
      : undefined;
  }, [schedulingKey, selectionKey, selectedTaskKey, ready, revision]);

  useEffect(() => {
    if (!enabled) {
      const changed = jobs.current.size > 0 || completed.current.size > 0 || attempted.current.size > 0;
      jobs.current.forEach(stopJob); jobs.current.clear(); completed.current.clear(); attempted.current.clear();
      remembered.current.targets.clear(); handledManualRevision.current = manualRevision;
      if (changed) setRevision(value => value + 1);
      return;
    }
    const live = current.current;
    const manual = handledManualRevision.current !== manualRevision;
    handledManualRevision.current = manualRevision;
    let changed = false;
    for (const [task, job] of jobs.current) {
      const target = live.targets.find(candidate => candidate.task === task);
      if (!live.ready || !target || job.key !== probeKey(live.request, target) || manual ||
        (!job.dispatched && !job.manual && !live.automaticTasks.includes(task))) {
        stopJob(job); jobs.current.delete(task); changed = true;
      }
    }
    if (live.ready) for (const target of live.targets) {
      if (jobs.current.has(target.task) || (!manual && (attempted.current.has(target.task) || !live.automaticTasks.includes(target.task)))) continue;
      const source = live.request;
      const job: ProbeJob = { key: probeKey(source, target), controller: new AbortController(), dispatched: false, manual };
      jobs.current.set(target.task, job); changed = true;
      job.timer = globalThis.setTimeout(() => {
        const latest = current.current;
        const latestTarget = latest.targets.find(candidate => candidate.task === target.task);
        if (!latest.ready || !latestTarget || jobs.current.get(target.task) !== job || probeKey(latest.request, latestTarget) !== job.key) return;
        job.dispatched = true;
        attempted.current.add(target.task);
        setRevision(value => value + 1);
        void (async () => {
          const attempts = await Promise.all(target.candidates.map(async (candidate): Promise<ProviderAutoConfigurationProbeAttempt> => {
            try {
              const response = source.providerId !== undefined
                ? await ipcBridge.mode.probeProviderConnection.invoke({ provider_id: source.providerId,
                    protocol: candidate.descriptor.protocol_id, task: target.task, probe_candidates: true }, job.controller.signal)
                : await ipcBridge.mode.probeConnection.invoke({ platform: source.platform, base_url: source.baseUrl,
                    auth_scheme: candidate.authScheme, credentials: source.credentials ?? {},
                    protocol: candidate.descriptor.protocol_id, task: target.task, probe_candidates: true }, job.controller.signal);
              return { candidate, response };
            } catch { return { candidate }; }
          }));
          const active = current.current;
          const activeTarget = active.targets.find(candidate => candidate.task === target.task);
          if (job.controller.signal.aborted || jobs.current.get(target.task) !== job || !active.ready || !activeTarget ||
            probeKey(active.request, activeTarget) !== job.key) return;
          const validKeys = new Set([job.key]);
          // Applying a tested candidate can update this task's root/auth. Those
          // values still refer to this exact check, including intermediate form updates.
          for (const attempt of attempts) {
            const detection = selectProviderAutoConfiguration({ task: target.task, candidates: [attempt.candidate] }, [attempt]);
            for (const root of new Set([source.baseUrl, detection?.suggestedBaseUrl ?? source.baseUrl])) {
              for (const scheme of new Set([source.authScheme, detection?.authScheme ?? source.authScheme])) {
                const aliasTarget = { ...target, candidates: target.candidates.flatMap(candidate => {
                  const aliasScheme = source.providerId !== undefined ? scheme : selectProbeAuthScheme(candidate.descriptor, scheme);
                  return aliasScheme ? [{ ...candidate, authScheme: aliasScheme }] : [];
                }) };
                validKeys.add(probeKey(source, aliasTarget, root, scheme));
              }
            }
          }
          completed.current.set(target.task, { requestBaseUrl: source.baseUrl, target, attempts, validKeys });
          jobs.current.delete(target.task);
          setRevision(value => value + 1);
        })();
      }, manual ? 0 : PROBE_DEBOUNCE_MS);
    }
    if (changed) setRevision(value => value + 1);
  }, [enabled, schedulingKey, ready, manualRevision]);

  useEffect(() => () => { jobs.current.forEach(stopJob); jobs.current.clear(); }, []);
  const isLoading = ready && targets.some(target => jobs.current.get(target.task)?.key === probeKey(request, target) ||
    (!attempted.current.has(target.task) && automaticTasks.includes(target.task)));
  const canRecheck = ready && targets.length > 0;
  const recheck = useCallback(() => {
    if (current.current.ready && current.current.targets.length > 0) setManualRevision(value => value + 1);
  }, []);
  return { data, isLoading, stale: Boolean(data?.staleTasks?.length), canRecheck, recheck };
};

export default useProviderAutoConfiguration;
