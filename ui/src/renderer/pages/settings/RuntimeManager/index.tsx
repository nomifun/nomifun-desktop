/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  JavaScriptRuntimeProbe,
  JavaScriptRuntimeStatus,
  RuntimeSwitchDecision,
} from '@/common/types/javascriptRuntime';
import { Button, Alert, Modal, Spin } from '@arco-design/web-react';
import {
  CheckOne,
  CloseOne,
  Download,
  FolderOpen,
  Refresh,
} from '@icon-park/react';
import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { javascriptRuntime } from '@/common/adapter/javascriptRuntimeBridge';
import { isDesktopShell } from '@/renderer/utils/platform';
import {
  buildBeginRuntimeSwitchRequest,
  buildRuntimeSwitchDecisionRequest,
  probeForManagedOffer,
  probeForRuntime,
  projectRuntimeCandidates,
  requiresNonRecommendedConfirmation,
  runtimeRefMatches,
  runtimeStatusNeedsPolling,
  type RuntimeCandidateProjection,
} from './runtimeManagerModel';

type RuntimeAction = 'refresh' | 'probe' | 'download' | 'switch' | 'decision' | null;

const shortDigest = (digest: string): string =>
  digest.length > 16 ? `${digest.slice(0, 8)}…${digest.slice(-8)}` : digest;

const errorText = (error: unknown): string => {
  if (isBackendHttpError(error)) {
    const detail = error.backendMessage || error.message;
    return error.code ? `${error.code}: ${detail}` : detail;
  }
  return error instanceof Error ? error.message : String(error);
};

const sourceLabel = (
  source: JavaScriptRuntimeProbe['source'],
  t: ReturnType<typeof useTranslation>['t']
): string => {
  const labels: Record<JavaScriptRuntimeProbe['source'], string> = {
    manual_path: t('settings.runtimeManager.source.manualPath'),
    process_path: t('settings.runtimeManager.source.processPath'),
    managed: t('settings.runtimeManager.source.managed'),
  };
  return labels[source];
};

const compatibilityLabel = (
  compatibility: JavaScriptRuntimeProbe['compatibility'],
  t: ReturnType<typeof useTranslation>['t']
): string => {
  const labels: Record<JavaScriptRuntimeProbe['compatibility'], string> = {
    recommended: t('settings.runtimeManager.compatibility.recommended'),
    compatible: t('settings.runtimeManager.compatibility.compatible'),
    incompatible: t('settings.runtimeManager.compatibility.incompatible'),
  };
  return labels[compatibility];
};

const RuntimeBadge: React.FC<{
  label: string;
  tone: 'success' | 'warning' | 'danger' | 'muted' | 'info';
}> = ({ label, tone }) => {
  const classes: Record<typeof tone, string> = {
    success: 'bg-success-1 text-success-6',
    warning: 'bg-warning-1 text-warning-6',
    danger: 'bg-danger-1 text-danger-6',
    muted: 'bg-fill-2 text-t-tertiary',
    info: 'bg-primary-1 text-primary-6',
  };
  return (
    <span
      className={`inline-flex shrink-0 items-center rd-999px px-8px py-2px text-11px leading-16px font-500 ${classes[tone]}`}
    >
      {label}
    </span>
  );
};

const compatibilityTone = (
  compatibility: JavaScriptRuntimeProbe['compatibility']
): 'success' | 'warning' | 'danger' => {
  if (compatibility === 'recommended') return 'success';
  if (compatibility === 'compatible') return 'warning';
  return 'danger';
};

const RuntimeIdentity: React.FC<{
  status: JavaScriptRuntimeStatus;
  runtime: JavaScriptRuntimeStatus['selected'];
  emptyLabel: string;
  t: ReturnType<typeof useTranslation>['t'];
}> = ({ status, runtime, emptyLabel, t }) => {
  const probe = probeForRuntime(status, runtime);
  if (!runtime) {
    return <span className='text-13px text-t-tertiary'>{emptyLabel}</span>;
  }

  return (
    <div className='min-w-0 flex flex-col gap-3px'>
      <div className='flex flex-wrap items-center gap-6px'>
        <span className='text-14px font-600 text-t-primary'>{runtime.node_version}</span>
        {probe && <RuntimeBadge label={sourceLabel(probe.source, t)} tone='info' />}
        {probe && (
          <RuntimeBadge
            label={compatibilityLabel(probe.compatibility, t)}
            tone={compatibilityTone(probe.compatibility)}
          />
        )}
      </div>
      <span className='break-all text-12px leading-18px text-t-secondary'>
        {probe?.executable_path ?? emptyLabel}
      </span>
      <span className='break-all font-mono text-11px leading-16px text-t-tertiary'>
        {runtime.runtime_target} · {shortDigest(runtime.executable_digest)}
      </span>
    </div>
  );
};

const ParticipantList: React.FC<{
  status: JavaScriptRuntimeStatus;
  t: ReturnType<typeof useTranslation>['t'];
}> = ({ status, t }) => {
  if (status.switch_participants.length === 0) {
    return (
      <div className='text-12px leading-18px text-t-tertiary'>
        {t('settings.runtimeManager.switch.noParticipants')}
      </div>
    );
  }

  const kindLabels: Record<
    JavaScriptRuntimeStatus['switch_participants'][number]['kind'],
    string
  > = {
    plugin_mount: t('settings.runtimeManager.switch.participantPlugin'),
    miniapp_service: t('settings.runtimeManager.switch.participantPluginRuntime'),
    build_foundation: t('settings.runtimeManager.switch.participantBuild'),
  };

  return (
    <div className='flex flex-col gap-6px'>
      {status.switch_participants.map((participant) => {
        const passed = participant.status === 'passed';
        const notCovered = participant.status === 'not_covered';
        return (
          <div
            key={`${participant.kind}:${participant.owner_id}`}
            className='flex min-w-0 items-center gap-8px border-b border-b-solid border-b-[var(--color-border-2)] pb-6px last:border-b-0 last:pb-0'
          >
            <span className='shrink-0'>
              {passed ? (
                <CheckOne theme='outline' size='16' className='text-success-6' />
              ) : (
                <CloseOne
                  theme='outline'
                  size='16'
                  className={notCovered ? 'text-warning-6' : 'text-danger-6'}
                />
              )}
            </span>
            <span className='min-w-0 flex-1'>
              <span className='block text-12px text-t-primary'>
                {kindLabels[participant.kind]}
              </span>
              <span className='block truncate font-mono text-11px text-t-tertiary' title={participant.owner_id}>
                {participant.owner_id}
              </span>
            </span>
            <RuntimeBadge
              label={
                passed
                  ? t('settings.runtimeManager.switch.passed')
                  : notCovered
                    ? t('settings.runtimeManager.switch.notCovered')
                    : t('settings.runtimeManager.switch.failed')
              }
              tone={passed ? 'success' : notCovered ? 'warning' : 'danger'}
            />
          </div>
        );
      })}
    </div>
  );
};

const CandidateRow: React.FC<{
  item: RuntimeCandidateProjection;
  action: RuntimeAction;
  switchingRuntimeId: string | null;
  onUse: (probe: JavaScriptRuntimeProbe) => void;
  t: ReturnType<typeof useTranslation>['t'];
}> = ({ item, action, switchingRuntimeId, onUse, t }) => {
  const { probe } = item;
  return (
    <div
      className={`flex min-w-0 flex-col gap-10px border-b border-b-solid border-b-[var(--color-border-2)] px-10px py-10px last:border-b-0 ${
        item.selected ? 'bg-primary-1' : 'bg-[var(--color-bg-2)]'
      }`}
    >
      <div className='flex min-w-0 items-start gap-10px'>
        <div className='min-w-0 flex-1'>
          <div className='flex flex-wrap items-center gap-6px'>
            <span className='text-13px font-600 text-t-primary'>
              {probe.runtime?.node_version ?? t('settings.runtimeManager.unknownVersion')}
            </span>
            <RuntimeBadge label={sourceLabel(probe.source, t)} tone='info' />
            <RuntimeBadge
              label={compatibilityLabel(probe.compatibility, t)}
              tone={compatibilityTone(probe.compatibility)}
            />
            {item.selected && (
              <RuntimeBadge
                label={t('settings.runtimeManager.candidate.selected')}
                tone='success'
              />
            )}
            {item.pending && (
              <RuntimeBadge
                label={t('settings.runtimeManager.candidate.pending')}
                tone='warning'
              />
            )}
          </div>
          <div className='mt-4px break-all text-12px leading-18px text-t-secondary'>
            {probe.executable_path}
          </div>
          {probe.runtime && (
            <div className='mt-2px break-all font-mono text-11px leading-16px text-t-tertiary'>
              {probe.runtime.runtime_target} · {shortDigest(probe.runtime.executable_digest)}
            </div>
          )}
          {probe.error_code && (
            <div className='mt-4px text-11px leading-16px text-danger-6'>
              {probe.error_code}
            </div>
          )}
        </div>
        <Button
          size='small'
          type={item.selected ? 'secondary' : 'primary'}
          disabled={!item.selectable || action !== null}
          loading={
            action === 'switch' &&
            probe.runtime?.runtime_installation_id === switchingRuntimeId
          }
          onClick={() => onUse(probe)}
        >
          {item.selected
            ? t('settings.runtimeManager.candidate.current')
            : item.pending
              ? t('settings.runtimeManager.candidate.pending')
              : t('settings.runtimeManager.candidate.use')}
        </Button>
      </div>
    </div>
  );
};

const RuntimeManager: React.FC = () => {
  const { t } = useTranslation();
  const desktop = isDesktopShell();
  const [status, setStatus] = useState<JavaScriptRuntimeStatus | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [action, setAction] = useState<RuntimeAction>(desktop ? 'refresh' : null);
  const [switchingRuntimeId, setSwitchingRuntimeId] = useState<string | null>(null);
  const autoSwitchKey = useRef<string | null>(null);
  const active = useRef(false);
  const busy = useRef(false);
  const requestVersion = useRef(0);
  const confirmation = useRef<ReturnType<typeof Modal.confirm> | null>(null);
  const loading = action === 'refresh';

  useLayoutEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      busy.current = false;
      requestVersion.current++;
      confirmation.current?.close();
    };
  }, []);

  const applyStatus = useCallback((next: JavaScriptRuntimeStatus) => {
    setStatus(next);
    setFailure(null);
  }, []);

  // One foreground operation at a time; an older background poll cannot undo it.
  const runAction = useCallback(async (
    kind: Exclude<RuntimeAction, null>,
    request: () => Promise<JavaScriptRuntimeStatus | undefined>
  ) => {
    if (!desktop || !active.current || busy.current) return;
    busy.current = true;
    const version = ++requestVersion.current;
    const isCurrent = () => active.current && version === requestVersion.current;
    setAction(kind);
    setFailure(null);
    try {
      const next = await request();
      if (next && isCurrent()) applyStatus(next);
    } catch (error) {
      if (isCurrent()) {
        console.error('[runtime-manager] ' + kind + ' failed', error);
        setFailure(errorText(error));
      }
    } finally {
      if (isCurrent()) {
        busy.current = false;
        setAction(null);
        setSwitchingRuntimeId(null);
      }
    }
  }, [applyStatus, desktop]);

  const loadStatus = useCallback(() => runAction('refresh', () => javascriptRuntime.status.invoke()), [runAction]);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  const beginSwitch = useCallback(
    (candidate: JavaScriptRuntimeProbe, acknowledge: boolean, sourceStatus: JavaScriptRuntimeStatus) =>
      runAction('switch', () => {
        setSwitchingRuntimeId(candidate.runtime?.runtime_installation_id ?? null);
        return javascriptRuntime.beginSwitch.invoke(
          buildBeginRuntimeSwitchRequest(sourceStatus, candidate, acknowledge)
        );
      }),
    [runAction]
  );

  const handleUseCandidate = useCallback(
    (candidate: JavaScriptRuntimeProbe) => {
      if (!active.current || busy.current || !status || !candidate.runtime) return;
      if (requiresNonRecommendedConfirmation(status, candidate)) {
        confirmation.current?.close();
        confirmation.current = Modal.confirm({
          title: t('settings.runtimeManager.confirm.nonRecommendedTitle'),
          content: t('settings.runtimeManager.confirm.nonRecommendedBody', {
            version: candidate.runtime.node_version,
          }),
          okText: t('settings.runtimeManager.actions.continue'),
          cancelText: t('settings.runtimeManager.actions.cancel'),
          onOk: () => beginSwitch(candidate, true, status),
        });
        return;
      }
      void beginSwitch(candidate, false, status);
    },
    [beginSwitch, status, t]
  );

  const handleAutoDiscover = useCallback(() => {
    if (!status) return;
    void runAction('probe', () => javascriptRuntime.probe.invoke({
      source: 'auto_discover',
      expected_selection_revision: status.selection_revision,
    }));
  }, [runAction, status]);

  const handleChoosePath = useCallback(async () => {
    if (!status) return;
    await runAction('probe', async () => {
      const paths = await ipcBridge.dialog.showOpen.invoke({ properties: ['openFile'] });
      const executablePath = paths?.[0];
      if (!active.current || !executablePath) return;
      return javascriptRuntime.probe.invoke({
        source: 'manual_path',
        expected_selection_revision: status.selection_revision,
        executable_path: executablePath,
      });
    });
  }, [runAction, status]);

  const handleDownload = useCallback(async () => {
    const offer = status?.download_offer;
    if (!status || !offer) return;
    await runAction('download', () => {
      autoSwitchKey.current = null;
      return javascriptRuntime.download.invoke({
        expected_selection_revision: status.selection_revision,
        expected_offer_digest: offer.offer_digest,
      });
    });
  }, [runAction, status]);

  const confirmDownload = useCallback(() => {
    if (!active.current || busy.current || !status?.download_offer) return;
    confirmation.current?.close();
    confirmation.current = Modal.confirm({
      title: t('settings.runtimeManager.confirm.downloadTitle'),
      content: t('settings.runtimeManager.confirm.downloadBody', {
        version: status.download_offer.node_version,
        target: status.download_offer.runtime_target,
      }),
      okText: t('settings.runtimeManager.actions.download'),
      cancelText: t('settings.runtimeManager.actions.cancel'),
      onOk: handleDownload,
    });
  }, [handleDownload, status?.download_offer, t]);

  const handleDecision = useCallback(
    async (decision: RuntimeSwitchDecision) => {
      if (!status) return;
      await runAction('decision', () => javascriptRuntime.decideSwitch.invoke(
        buildRuntimeSwitchDecisionRequest(status, decision)
      ));
    },
    [runAction, status]
  );

  const downloadPolling = runtimeStatusNeedsPolling(status);

  useEffect(() => {
    if (!desktop || !downloadPolling || action !== null) return undefined;
    let cancelled = false;
    let timer: number | undefined;

    const poll = async (): Promise<void> => {
      const version = requestVersion.current;
      const isCurrent = () => !cancelled && active.current && !busy.current && version === requestVersion.current;
      if (!isCurrent()) return;
      try {
        const next = await javascriptRuntime.status.invoke();
        if (!isCurrent()) return;
        applyStatus(next);
        if (next.download.state === 'downloading') {
          timer = window.setTimeout(() => void poll(), 1500);
          return;
        }
        const downloadedRuntime = next.download.runtime;
        const downloadedProbe = probeForRuntime(next, downloadedRuntime);
        if (
          next.download.state === 'ready' &&
          downloadedRuntime &&
          downloadedProbe &&
          downloadedProbe.compatibility !== 'incompatible' &&
          !next.pending_candidate &&
          !runtimeRefMatches(downloadedRuntime, next.selected) &&
          autoSwitchKey.current !==
            `${downloadedRuntime.runtime_installation_id}:${downloadedRuntime.executable_digest}`
        ) {
          autoSwitchKey.current = `${downloadedRuntime.runtime_installation_id}:${downloadedRuntime.executable_digest}`;
          await beginSwitch(downloadedProbe, false, next);
        }
      } catch (error) {
        if (isCurrent()) {
          console.error('[runtime-manager] managed Node status poll failed', error);
          setFailure(errorText(error));
          timer = window.setTimeout(() => void poll(), 3000);
        }
      }
    };

    timer = window.setTimeout(() => void poll(), 1500);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [action, applyStatus, beginSwitch, desktop, downloadPolling]);

  const candidates = useMemo(
    () => (status ? projectRuntimeCandidates(status) : []),
    [status]
  );
  const selectedProbe = status ? probeForRuntime(status, status.selected) : undefined;
  const pendingProbe = status
    ? probeForRuntime(status, status.pending_candidate)
    : undefined;
  const managedProbe = status ? probeForManagedOffer(status) : undefined;
  const managedInstalled = Boolean(managedProbe);
  const managedIsSelected = runtimeRefMatches(managedProbe?.runtime, status?.selected);
  const managedIsPending = runtimeRefMatches(
    managedProbe?.runtime,
    status?.pending_candidate
  );
  const downloadReady = status?.download.state === 'ready';

  if (!desktop) {
    return (
      <Alert
        type='info'
        showIcon
        title={t('settings.runtimeManager.desktopOnlyTitle')}
        content={t('settings.runtimeManager.desktopOnlyBody')}
      />
    );
  }

  return (
    <div data-testid='runtime-manager' className='flex w-full flex-col gap-16px'>
      {failure && <Alert type='error' showIcon content={failure} />}
      {!failure && status?.last_error_code && (
        <Alert
          type='warning'
          showIcon
          content={t('settings.runtimeManager.statusError', {
            code: status.last_error_code,
          })}
        />
      )}

      <section className='flex flex-col gap-8px'>
        <div className='flex min-w-0 flex-wrap items-start justify-between gap-10px'>
          <div className='min-w-0'>
            <h2 className='m-0 text-15px leading-22px font-600 text-t-primary'>
              {t('settings.runtimeManager.currentTitle')}
            </h2>
            <p className='m-0 mt-3px text-12px leading-18px text-t-tertiary'>
              {t('settings.runtimeManager.currentDescription')}
            </p>
          </div>
          <Button
            size='small'
            icon={<Refresh theme='outline' size='14' />}
            loading={loading}
            disabled={action !== null}
            onClick={() => void loadStatus()}
          >
            {t('settings.runtimeManager.actions.refresh')}
          </Button>
        </div>
        <div className='overflow-hidden rd-8px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-12px py-10px'>
          {loading && !status ? (
            <div className='flex items-center gap-8px text-12px text-t-secondary'>
              <Spin size={16} />
              {t('settings.runtimeManager.loading')}
            </div>
          ) : (
            <RuntimeIdentity
              status={status ?? {
                selection_revision: 0,
                probes: [],
                switch_participants: [],
                requires_switch_decision: false,
                non_recommended_warning_acknowledged: [],
                download: { download_revision: 0, state: 'not_installed' },
              }}
              runtime={status?.selected}
              emptyLabel={t('settings.runtimeManager.noSelection')}
              t={t}
            />
          )}
        </div>
        {status && !status.selected && (
          <Alert
            type='warning'
            showIcon
            content={t('settings.runtimeManager.noSelectionBody')}
          />
        )}
      </section>

      <section className='flex flex-col gap-8px'>
        <div className='flex min-w-0 flex-wrap items-start justify-between gap-10px'>
          <div className='min-w-0'>
            <h2 className='m-0 text-15px leading-22px font-600 text-t-primary'>
              {t('settings.runtimeManager.candidatesTitle')}
            </h2>
            <p className='m-0 mt-3px text-12px leading-18px text-t-tertiary'>
              {t('settings.runtimeManager.candidatesDescription')}
            </p>
          </div>
          <div className='flex flex-wrap items-center justify-end gap-6px'>
            <Button
              size='small'
              icon={<FolderOpen theme='outline' size='14' />}
              disabled={action !== null}
              onClick={() => void handleChoosePath()}
            >
              {t('settings.runtimeManager.actions.choosePath')}
            </Button>
            <Button
              size='small'
              icon={<Refresh theme='outline' size='14' />}
              loading={action === 'probe'}
              disabled={action !== null}
              onClick={handleAutoDiscover}
            >
              {t('settings.runtimeManager.actions.scan')}
            </Button>
          </div>
        </div>
        <div className='overflow-hidden rd-8px border border-solid border-[var(--color-border-2)]'>
          {candidates.length === 0 ? (
            <div className='px-12px py-16px text-12px text-t-tertiary'>
              {t('settings.runtimeManager.candidatesEmpty')}
            </div>
          ) : (
            candidates.map((item) => (
              <CandidateRow
                key={item.key}
                item={item}
                action={action}
                switchingRuntimeId={switchingRuntimeId}
                onUse={handleUseCandidate}
                t={t}
              />
            ))
          )}
        </div>
      </section>

      <section className='flex flex-col gap-8px'>
        <div>
          <h2 className='m-0 text-15px leading-22px font-600 text-t-primary'>
            {t('settings.runtimeManager.downloadTitle')}
          </h2>
          <p className='m-0 mt-3px text-12px leading-18px text-t-tertiary'>
            {t('settings.runtimeManager.downloadDescription')}
          </p>
        </div>
        <div className='overflow-hidden rd-8px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-12px py-10px'>
          {status?.download_offer ? (
            <div className='flex min-w-0 flex-wrap items-center justify-between gap-10px'>
              <div className='min-w-0'>
                <div className='flex flex-wrap items-center gap-6px'>
                  <span className='text-13px font-600 text-t-primary'>
                    Node.js {status.download_offer.node_version}
                  </span>
                  <RuntimeBadge
                    label={t('settings.runtimeManager.source.managed')}
                    tone='info'
                  />
                </div>
                <div className='mt-3px break-all text-11px text-t-tertiary'>
                  {status.download_offer.archive_file_name}
                  {status.download_offer.archive_size_bytes
                    ? ` · ${Math.round(status.download_offer.archive_size_bytes / 1024 / 1024)} MB`
                    : ''}
                </div>
                {status.download.state === 'downloading' && (
                  <div className='mt-6px text-12px text-primary-6' aria-live='polite'>
                    {t('settings.runtimeManager.download.downloading')}
                  </div>
                )}
                {status.download.state === 'failed' && (
                  <div className='mt-6px text-12px text-danger-6'>
                    {status.download.error_code ?? t('settings.runtimeManager.download.failed')}
                  </div>
                )}
                {downloadReady && (
                  <div className='mt-6px text-12px text-success-6'>
                    {t('settings.runtimeManager.download.ready')}
                  </div>
                )}
              </div>
              <Button
                type='primary'
                size='small'
                icon={<Download theme='outline' size='14' />}
                loading={action === 'download'}
                disabled={
                  action !== null ||
                  status.download.state === 'downloading' ||
                  (managedInstalled && Boolean(status.pending_candidate)) ||
                  managedIsSelected
                }
                onClick={() => {
                  if (managedInstalled && managedProbe) {
                    handleUseCandidate(managedProbe);
                  } else {
                    confirmDownload();
                  }
                }}
              >
                {managedIsSelected
                  ? t('settings.runtimeManager.candidate.current')
                  : managedIsPending
                    ? t('settings.runtimeManager.candidate.pending')
                    : managedInstalled || downloadReady
                      ? t('settings.runtimeManager.actions.useDownloaded')
                      : t('settings.runtimeManager.actions.download')}
              </Button>
            </div>
          ) : (
            <span className='text-12px text-t-tertiary'>
              {t('settings.runtimeManager.download.unavailable')}
            </span>
          )}
        </div>
      </section>

      {status?.requires_switch_decision && status.pending_candidate && (
        <section className='flex flex-col gap-8px'>
          <div>
            <h2 className='m-0 text-15px leading-22px font-600 text-t-primary'>
              {t('settings.runtimeManager.switch.title')}
            </h2>
            <p className='m-0 mt-3px text-12px leading-18px text-t-tertiary'>
              {t('settings.runtimeManager.switch.description')}
            </p>
          </div>
          <div className='overflow-hidden rd-8px border border-solid border-warning-3 bg-warning-1 px-12px py-10px'>
            <div className='mb-10px'>
              <RuntimeIdentity
                status={status}
                runtime={status.pending_candidate}
                emptyLabel={t('settings.runtimeManager.noSelection')}
                t={t}
              />
            </div>
            <ParticipantList status={status} t={t} />
            <div className='mt-12px flex flex-wrap justify-end gap-6px'>
              <Button
                size='small'
                loading={action === 'decision'}
                disabled={action !== null}
                onClick={() => void handleDecision('abort_and_restore_selected')}
              >
                {t('settings.runtimeManager.actions.restore')}
              </Button>
              <Button
                type='primary'
                size='small'
                loading={action === 'decision'}
                disabled={action !== null}
                onClick={() => void handleDecision('commit_candidate')}
              >
                {t('settings.runtimeManager.actions.continue')}
              </Button>
            </div>
          </div>
        </section>
      )}

      {status && (
        <details className='text-12px text-t-tertiary'>
          <summary className='cursor-pointer select-none text-12px text-t-secondary'>
            {t('settings.runtimeManager.diagnostics')}
          </summary>
          <div className='mt-8px flex flex-col gap-4px rounded-8px bg-fill-1 px-10px py-8px font-mono text-11px leading-16px'>
            <span>selection_revision: {status.selection_revision}</span>
            <span>selected_probe: {selectedProbe?.executable_path ?? 'none'}</span>
            <span>pending_probe: {pendingProbe?.executable_path ?? 'none'}</span>
            <span>last_error_code: {status.last_error_code ?? 'none'}</span>
          </div>
        </details>
      )}
    </div>
  );
};

export default RuntimeManager;
