/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, Button, Empty, Message, Modal, Spin } from '@arco-design/web-react';
import { WebPage } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useSearchParams } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { createBrowserDisplayModeController } from '@/common/browser/browserDisplayModeController';
import {
  resolveBrowserOverviewCapabilities,
  type BrowserDisplayMode,
  type IBrowserLane,
} from '@/common/browser/browserTypes';
import { useConversationHistoryContext } from '@/renderer/hooks/context/ConversationHistoryContext';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import BrowserInventoryTree from './BrowserInventoryTree';
import BrowserDisplayModeControl, {
  type BrowserDisplayModeControlStatus,
} from './BrowserDisplayModeControl';
import BrowserLaneDetails from './BrowserLaneDetails';
import BrowserPageHeader from './BrowserPageHeader';
import {
  browserConversationSearchParamsForLane,
  browserLaneCounts,
  buildBrowserInventoryTree,
  matchBrowserLaneHost,
  pickDefaultBrowserLaneId,
  resolveBrowserConversationId,
  type BrowserConversationGroup,
} from './browserInventoryModel';
import {
  browserInstallationWideCloseCopy,
  browserClosePartialFailureMessage,
  canBackgroundBrowserLane,
  canForegroundBrowserLane,
  createBrowserManagementMutationGate,
  requestBrowserCloseAll,
  requestBrowserConversationClose,
  requestBrowserLaneClose,
  runBrowserCloseAll,
  runBrowserConversationClose,
  runBrowserLaneBackground,
  runBrowserLaneForeground,
  runBrowserLaneClose,
  type BrowserConfirmationRequest,
} from './browserManagementActions';
import { useBrowserInventory } from './useBrowserInventory';
import BrowserHostDiagnostics from './BrowserHostDiagnostics';

const displayModeErrorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const displayModeEndpointUnavailable = (error: unknown): boolean =>
  isBackendHttpError(error) && (error.status === 404 || error.status === 501);

const BrowserPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const location = useLocation();
  const layout = useLayoutContext();
  const { conversations } = useConversationHistoryContext();
  const { lanes, overview, loading, refreshing, error, refresh } = useBrowserInventory();
  const [selectedLaneId, setSelectedLaneId] = useState<string | null>(null);
  const [busyLaneId, setBusyLaneId] = useState<string | null>(null);
  const [changingVisibilityLaneId, setChangingVisibilityLaneId] = useState<string | null>(null);
  const [busyConversationId, setBusyConversationId] = useState<string | null>(null);
  const [closingAll, setClosingAll] = useState(false);
  const [displayMode, setDisplayMode] = useState<BrowserDisplayMode>('headless');
  const [displayModeStatus, setDisplayModeStatus] =
    useState<BrowserDisplayModeControlStatus>('loading');
  const [displayModeSaving, setDisplayModeSaving] = useState(false);
  const [displayModeError, setDisplayModeError] = useState<string | null>(null);
  const mutationGateRef = useRef(createBrowserManagementMutationGate());
  const displayModeControllerRef = useRef(
    createBrowserDisplayModeController({
      get: () => ipcBridge.browserSession.displayMode.get.invoke(),
      put: (next) =>
        ipcBridge.browserSession.displayMode.put.invoke({ display_mode: next }),
    })
  );

  const installationWideCloseCopy = browserInstallationWideCloseCopy(
    i18n.resolvedLanguage ?? i18n.language
  );
  const requestedConversationId = searchParams.get('conversation_id');
  const currentConversationId = useMemo(
    () =>
      resolveBrowserConversationId({
        requestedConversationId,
        pathname: location.pathname,
        locationState: location.state,
      }),
    [location.pathname, location.state, requestedConversationId]
  );

  const conversationNames = useMemo(
    () =>
      Object.fromEntries(
        conversations.map((conversation) => [String(conversation.id), conversation.name])
      ),
    [conversations]
  );
  const inventoryLabels = useMemo(
    () => ({
      clusterNode: (id: string) => t('browser.fallback.clusterNode', { id }),
      attempt: (id: string) => t('browser.fallback.attempt', { id }),
      runtime: (id: string) => t('browser.fallback.runtime', { id }),
      execution: (id: string) => t('browser.fallback.execution', { id }),
      owner: t('browser.fallback.owner'),
      laneOwner: t('browser.fallback.laneOwner'),
      conversation: (id: string) => t('browser.fallback.conversation', { id }),
      unassigned: t('browser.fallback.unassigned'),
    }),
    [t]
  );
  const groups = useMemo(
    () =>
      buildBrowserInventoryTree(
        lanes,
        conversationNames,
        inventoryLabels,
        currentConversationId
      ),
    [lanes, conversationNames, inventoryLabels, currentConversationId]
  );
  const selectedLane = useMemo(
    () => lanes.find((lane) => lane.lane_id === selectedLaneId) ?? null,
    [lanes, selectedLaneId]
  );
  const localCounts = useMemo(() => browserLaneCounts(lanes), [lanes]);
  const runningCount = overview?.running_lanes ?? localCounts.running;
  const queuedCount = overview?.queued_lanes ?? localCounts.queued;
  const { canCloseAll, canManageBrowserSettings } =
    resolveBrowserOverviewCapabilities(overview);
  const managedHostCount =
    overview?.managed_host_count ?? overview?.hosts?.length ?? 0;
  const pendingCleanupCount = overview?.pending_cleanup_count ?? 0;
  const hasManagedResources =
    lanes.length > 0 || managedHostCount > 0 || pendingCleanupCount > 0;
  const hasResidualResources = lanes.length === 0 && hasManagedResources;
  // Lanes and overview come from two independent requests; the shared helper
  // matches by stable host_id first and tolerates one-epoch snapshot skew.
  const selectedLaneHost = useMemo(
    () => matchBrowserLaneHost(selectedLane, overview?.hosts),
    [overview?.hosts, selectedLane]
  );
  const managementMutationBusy =
    busyLaneId != null ||
    busyConversationId != null ||
    changingVisibilityLaneId != null ||
    closingAll ||
    displayModeSaving;
  const runManagementMutation = useCallback(
    async (operation: () => Promise<void>): Promise<void> => {
      await mutationGateRef.current.run(operation, () =>
        Message.warning(t('browser.actions.busy'))
      );
    },
    [t]
  );

  useEffect(() => {
    const requestedGroup = currentConversationId
      ? groups.find((group) => group.conversationId === currentConversationId)
      : null;
    const selectedStillValid = lanes.some((lane) => lane.lane_id === selectedLaneId);
    const selectedInRequested =
      !requestedGroup ||
      requestedGroup.lanes.some((lane) => lane.lane_id === selectedLaneId);
    if (!selectedStillValid || !selectedInRequested) {
      setSelectedLaneId(pickDefaultBrowserLaneId(groups, currentConversationId));
    }
  }, [groups, lanes, currentConversationId, selectedLaneId]);

  const loadDisplayMode = useCallback(async () => {
    if (!canManageBrowserSettings) return;
    setDisplayModeStatus('loading');
    setDisplayModeError(null);
    const result = await displayModeControllerRef.current.load();
    if (result.kind === 'applied') {
      setDisplayMode(result.displayMode);
      setDisplayModeStatus('ready');
    } else if (result.kind === 'error') {
      setDisplayModeStatus(
        displayModeEndpointUnavailable(result.error) ? 'unavailable' : 'error'
      );
      setDisplayModeError(
        displayModeEndpointUnavailable(result.error)
          ? null
          : displayModeErrorMessage(result.error)
      );
    }
  }, [canManageBrowserSettings]);

  useEffect(() => {
    if (canManageBrowserSettings) {
      void loadDisplayMode();
    }
  }, [canManageBrowserSettings, loadDisplayMode]);

  useEffect(
    () => () => displayModeControllerRef.current.dispose(),
    []
  );

  const refreshAll = useCallback(async () => {
    await Promise.all([
      refresh(),
      canManageBrowserSettings ? loadDisplayMode() : Promise.resolve(),
    ]);
  }, [canManageBrowserSettings, loadDisplayMode, refresh]);

  const handleSelectLane = useCallback(
    (lane: IBrowserLane) => {
      setSelectedLaneId(lane.lane_id);
      const next = browserConversationSearchParamsForLane(searchParams, lane);
      if (next.toString() !== searchParams.toString()) {
        setSearchParams(next, { replace: true });
      }
    },
    [searchParams, setSearchParams]
  );

  const handleDisplayModeChange = useCallback(
    async (next: BrowserDisplayMode) => {
      if (
        !canManageBrowserSettings ||
        displayModeStatus !== 'ready' ||
        managementMutationBusy ||
        mutationGateRef.current.isBusy() ||
        next === displayMode
      ) {
        return;
      }
      await runManagementMutation(async () => {
        setDisplayModeSaving(true);
        setDisplayModeError(null);
        try {
          const result = await displayModeControllerRef.current.save(next);
          if (result.kind === 'applied') {
            setDisplayMode(result.displayMode);
            setDisplayModeStatus('ready');
            Message.success(t('browser.displayMode.saved'));
            if (result.verificationError) {
              Message.error(
                t('browser.displayMode.refreshFailed', {
                  error: displayModeErrorMessage(result.verificationError),
                })
              );
            }
          } else if (result.kind === 'rejected') {
            const message = result.nonPersistent
              ? displayModeErrorMessage(result.error)
              : result.unconfirmed
                ? t('browser.displayMode.unconfirmed')
                : displayModeErrorMessage(result.error);
            setDisplayMode(result.displayMode);
            setDisplayModeStatus('ready');
            setDisplayModeError(message);
            Message.error(t('browser.displayMode.saveFailed', { error: message }));
          } else if (result.kind === 'unknown') {
            const message = displayModeErrorMessage(result.error);
            setDisplayModeStatus('error');
            setDisplayModeError(message);
            Message.error(t('browser.displayMode.saveFailed', { error: message }));
          }
          if (result.kind === 'applied' || result.kind === 'rejected') {
            try {
              await refresh();
            } catch (refreshError) {
              Message.error(
                t('browser.displayMode.refreshFailed', {
                  error: displayModeErrorMessage(refreshError),
                })
              );
            }
          }
        } finally {
          setDisplayModeSaving(false);
        }
      });
    },
    [
      canManageBrowserSettings,
      displayMode,
      displayModeStatus,
      managementMutationBusy,
      refresh,
      runManagementMutation,
      t,
    ]
  );

  const confirmDanger = useCallback((request: BrowserConfirmationRequest) => {
    Modal.confirm({
      ...request,
      okButtonProps: { status: 'danger' },
    });
  }, []);

  const closeLane = useCallback(
    (lane: IBrowserLane) =>
      runBrowserLaneClose(lane, {
        invoke: (request) => ipcBridge.browserSession.closeLane.invoke(request),
        refresh,
        setBusyLaneId,
        notifySuccess: Message.success,
        notifyError: Message.error,
        successMessage: t('browser.close.laneSuccess'),
        formatPartialFailure: (result) =>
          browserClosePartialFailureMessage(result, {
            withoutDetails: t('browser.close.partialFailure'),
            withDetails: (details) =>
              t('browser.close.partialFailureWithDetails', { details }),
          }),
        formatRefreshFailure: (message) =>
          t('browser.close.refreshFailed', { error: message }),
        unconfirmedMessage: t('browser.close.unconfirmed'),
      }),
    [refresh, t]
  );
  const closeLaneExclusively = useCallback(
    (lane: IBrowserLane) =>
      runManagementMutation(() => closeLane(lane)),
    [closeLane, runManagementMutation]
  );

  const handleCloseLane = useCallback(
    (lane: IBrowserLane) => {
      if (managementMutationBusy) return;
      void requestBrowserLaneClose(lane, closeLaneExclusively, confirmDanger, {
        title: t('browser.close.laneActiveTitle'),
        content: t('browser.close.laneActiveContent'),
        okText: t('browser.close.laneAction'),
        cancelText: t('browser.close.keepOpen'),
      });
    },
    [closeLaneExclusively, confirmDanger, managementMutationBusy, t]
  );

  const foregroundLane = useCallback(
    (lane: IBrowserLane) =>
      runBrowserLaneForeground(lane, {
        invoke: (request) => ipcBridge.browserSession.foregroundLane.invoke(request),
        refresh,
        setChangingVisibilityLaneId,
        notifySuccess: Message.success,
        notifyError: Message.error,
        successMessage: t('browser.foreground.success'),
        formatRefreshFailure: (message) =>
          t('browser.foreground.refreshFailed', { error: message }),
        unconfirmedMessage: t('browser.foreground.unconfirmed'),
      }),
    [refresh, t]
  );

  const handleForegroundLane = useCallback(
    (lane: IBrowserLane) => {
      if (managementMutationBusy) return;
      void runManagementMutation(() => foregroundLane(lane));
    },
    [foregroundLane, managementMutationBusy, runManagementMutation]
  );

  const backgroundLane = useCallback(
    (lane: IBrowserLane) =>
      runBrowserLaneBackground(lane, {
        invoke: (request) => ipcBridge.browserSession.backgroundLane.invoke(request),
        refresh,
        setChangingVisibilityLaneId,
        notifySuccess: Message.success,
        notifyError: Message.error,
        successMessage: t('browser.background.success'),
        formatRefreshFailure: (message) =>
          t('browser.background.refreshFailed', { error: message }),
        unconfirmedMessage: t('browser.background.unconfirmed'),
      }),
    [refresh, t]
  );

  const handleBackgroundLane = useCallback(
    (lane: IBrowserLane) => {
      if (managementMutationBusy) return;
      void runManagementMutation(() => backgroundLane(lane));
    },
    [backgroundLane, managementMutationBusy, runManagementMutation]
  );

  const closeConversation = useCallback(
    (conversationId: string) =>
      runBrowserConversationClose(conversationId, {
        invoke: (request) => ipcBridge.browserSession.closeConversation.invoke(request),
        refresh,
        setBusyConversationId,
        notifySuccess: Message.success,
        notifyError: Message.error,
        successMessage: t('browser.close.conversationSuccess'),
        formatPartialFailure: (result) =>
          browserClosePartialFailureMessage(result, {
            withoutDetails: t('browser.close.partialFailure'),
            withDetails: (details) =>
              t('browser.close.partialFailureWithDetails', { details }),
          }),
        formatRefreshFailure: (message) =>
          t('browser.close.refreshFailed', { error: message }),
        unconfirmedMessage: t('browser.close.unconfirmed'),
      }),
    [refresh, t]
  );
  const closeConversationExclusively = useCallback(
    (conversationId: string) =>
      runManagementMutation(() => closeConversation(conversationId)),
    [closeConversation, runManagementMutation]
  );

  const handleCloseConversation = useCallback(
    (group: BrowserConversationGroup) => {
      if (managementMutationBusy) return;
      requestBrowserConversationClose(
        group,
        closeConversationExclusively,
        confirmDanger,
        {
          title: t('browser.close.conversationTitle'),
          content: t('browser.close.conversationContent', {
            count: group.lanes.length,
          }),
          okText: t('browser.close.conversationAction'),
          cancelText: t('browser.close.keepOpen'),
        }
      );
    },
    [closeConversationExclusively, confirmDanger, managementMutationBusy, t]
  );

  const closeAll = useCallback(
    () =>
      runBrowserCloseAll({
        invoke: () => ipcBridge.browserSession.closeAll.invoke(),
        refresh,
        setClosingAll,
        notifySuccess: Message.success,
        notifyError: Message.error,
        successMessage: installationWideCloseCopy.success,
        formatPartialFailure: (result) =>
          browserClosePartialFailureMessage(result, {
            withoutDetails: t('browser.close.partialFailure'),
            withDetails: (details) =>
              t('browser.close.partialFailureWithDetails', { details }),
          }),
        formatRefreshFailure: (message) =>
          t('browser.close.refreshFailed', { error: message }),
        unconfirmedMessage: t('browser.close.drainUnconfirmed'),
      }),
    [installationWideCloseCopy.success, refresh, t]
  );
  const closeAllExclusively = useCallback(
    () => runManagementMutation(closeAll),
    [closeAll, runManagementMutation]
  );

  const handleCloseAll = useCallback(() => {
    if (managementMutationBusy) return;
    requestBrowserCloseAll(closeAllExclusively, confirmDanger, {
      title: installationWideCloseCopy.title,
      content: `${installationWideCloseCopy.warning} ${t('browser.close.allContent')}`,
      okText: installationWideCloseCopy.action,
      cancelText: t('browser.close.cancel'),
    });
  }, [
    closeAllExclusively,
    confirmDanger,
    installationWideCloseCopy,
    managementMutationBusy,
    t,
  ]);

  const capabilityUnavailable = overview?.supported === false || overview?.enabled === false;

  return (
    <div className='size-full min-h-0 flex flex-col p-16px box-border bg-bg-2'>
      <BrowserPageHeader
        runningCount={runningCount}
        queuedCount={queuedCount}
        pressureState={overview?.pressure_state}
        refreshing={refreshing}
        closingAll={closingAll}
        hasManagedResources={hasManagedResources}
        controlsDisabled={managementMutationBusy}
        canCloseAll={canCloseAll}
        closeAllLabel={installationWideCloseCopy.button}
        onRefresh={() => void refreshAll()}
        onCloseAll={handleCloseAll}
      />

      {error && (
        <Alert
          type='warning'
          showIcon
          className='mb-12px shrink-0'
          content={t('browser.page.inventoryUnavailable', { error })}
          action={
            <Button size='mini' onClick={() => void refreshAll()}>
              {t('browser.page.retry')}
            </Button>
          }
        />
      )}

      {canManageBrowserSettings && !capabilityUnavailable && (
        <BrowserDisplayModeControl
          displayMode={displayMode}
          status={displayModeStatus}
          saving={displayModeSaving}
          disabled={managementMutationBusy && !displayModeSaving}
          error={displayModeError}
          onChange={(next) => void handleDisplayModeChange(next)}
        />
      )}

      {overview && !capabilityUnavailable && <BrowserHostDiagnostics overview={overview} />}

      {capabilityUnavailable ? (
        <div className='flex-1 min-h-0 flex items-center justify-center bg-bg-1 rd-12px border border-solid border-[var(--color-border-2)]'>
          <Empty
            icon={<WebPage theme='outline' size='42' />}
            description={t('browser.page.capabilityUnavailable')}
          />
        </div>
      ) : loading ? (
        <div className='flex-1 min-h-0 flex items-center justify-center'>
          <Spin tip={t('browser.page.loading')} />
        </div>
      ) : hasResidualResources ? (
        <div className='flex-1 min-h-0 flex items-center justify-center bg-bg-1 rd-12px border border-solid border-[var(--color-border-2)] p-20px'>
          <Alert
            type='warning'
            showIcon
            content={t(
              canCloseAll
                ? 'browser.page.residualResources'
                : 'browser.page.residualResourcesUser',
              {
                hosts: managedHostCount,
                cleanups: pendingCleanupCount,
              }
            )}
          />
        </div>
      ) : lanes.length === 0 ? (
        <div className='flex-1 min-h-0 flex items-center justify-center bg-bg-1 rd-12px border border-solid border-[var(--color-border-2)]'>
          <Empty
            icon={<WebPage theme='outline' size='42' />}
            description={t('browser.page.empty')}
          />
        </div>
      ) : (
        <div
          className={
            layout?.isMobile
              ? 'flex-1 min-h-0 flex flex-col gap-12px overflow-y-auto'
              : 'flex-1 min-h-0 grid grid-cols-[320px_minmax(0,1fr)] gap-12px'
          }
        >
          <aside
            className={
              layout?.isMobile
                ? 'shrink-0 rd-14px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_72%,transparent)] bg-[color:color-mix(in_srgb,var(--color-bg-1)_76%,transparent)] p-8px'
                : 'min-h-0 overflow-y-auto rd-14px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_72%,transparent)] bg-[color:color-mix(in_srgb,var(--color-bg-1)_76%,transparent)] p-8px pr-6px shadow-[0_6px_20px_rgba(15,23,42,0.025)]'
            }
            aria-label={t('browser.page.inventoryAria')}
          >
            <BrowserInventoryTree
              groups={groups}
              selectedLaneId={selectedLaneId}
              currentConversationId={currentConversationId}
              busyLaneId={busyLaneId}
              busyConversationId={busyConversationId}
              managementDisabled={managementMutationBusy}
              onSelectLane={handleSelectLane}
              onCloseLane={handleCloseLane}
              onCloseConversation={handleCloseConversation}
            />
          </aside>
          <main className={layout?.isMobile ? 'min-h-0' : 'min-h-0 overflow-y-auto pr-2px'}>
            {selectedLane ? (
              <BrowserLaneDetails
                lane={selectedLane}
                closing={busyLaneId === selectedLane.lane_id || closingAll}
                visibilityChanging={
                  changingVisibilityLaneId === selectedLane.lane_id
                }
                actionsDisabled={
                  busyConversationId != null ||
                  displayModeSaving ||
                  (changingVisibilityLaneId != null &&
                    changingVisibilityLaneId !== selectedLane.lane_id) ||
                  (busyLaneId != null && busyLaneId !== selectedLane.lane_id)
                }
                hostHeadful={selectedLaneHost?.headful}
                canChangeVisibility={
                  selectedLaneHost?.headful === true
                    ? canBackgroundBrowserLane(selectedLane)
                    : canForegroundBrowserLane(selectedLane)
                }
                onClose={handleCloseLane}
                onForeground={handleForegroundLane}
                onBackground={handleBackgroundLane}
              />
            ) : (
              <Empty description={t('browser.page.selectLane')} />
            )}
          </main>
        </div>
      )}
    </div>
  );
};

export default BrowserPage;
