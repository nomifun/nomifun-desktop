/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, Button, Empty, Message, Modal, Spin, Tabs } from '@arco-design/web-react';
import { WebPage } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useSearchParams } from 'react-router-dom';
import { ipcBridge } from '@/common';
import {
  resolveBrowserOverviewCapabilities,
  type IBrowserLane,
} from '@/common/browser/browserTypes';
import BrowserUseSettingsContent from '@/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent';
import { useConversationHistoryContext } from '@/renderer/hooks/context/ConversationHistoryContext';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import BrowserInventoryTree from './BrowserInventoryTree';
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

type BrowserManagementTab = 'lifecycle' | 'settings';

export const resolveBrowserManagementTab = (value: string | null): BrowserManagementTab =>
  value === 'settings' ? 'settings' : 'lifecycle';

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
  const mutationGateRef = useRef(createBrowserManagementMutationGate());

  const installationWideCloseCopy = browserInstallationWideCloseCopy(
    i18n.resolvedLanguage ?? i18n.language
  );
  const activeTab = resolveBrowserManagementTab(searchParams.get('tab'));
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
  const { canCloseAll } = resolveBrowserOverviewCapabilities(overview);
  const managedHostCount =
    overview?.managed_host_count ?? overview?.hosts?.length ?? 0;
  const pendingCleanupCount = overview?.pending_cleanup_count ?? 0;
  const hasManagedResources =
    lanes.length > 0 || managedHostCount > 0 || pendingCleanupCount > 0;
  const hasResidualResources = lanes.length === 0 && hasManagedResources;
  // Lanes and overview come from two independent requests; the shared helper
  // matches by browser epoch and tolerates one-epoch snapshot skew.
  const selectedLaneHost = useMemo(
    () => matchBrowserLaneHost(selectedLane, overview?.hosts),
    [overview?.hosts, selectedLane]
  );
  const managementMutationBusy =
    busyLaneId != null ||
    busyConversationId != null ||
    changingVisibilityLaneId != null ||
    closingAll;
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

  const refreshAll = useCallback(async () => {
    await refresh();
  }, [refresh]);

  const handleTabChange = useCallback(
    (key: string) => {
      const next = new URLSearchParams(searchParams);
      if (key === 'settings') next.set('tab', 'settings');
      else next.delete('tab');
      setSearchParams(next, { replace: true });
    },
    [searchParams, setSearchParams]
  );

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
    <div className='size-full min-h-0 flex flex-col p-12px box-border bg-2 overflow-hidden'>
      <BrowserPageHeader
        runningCount={runningCount}
        queuedCount={queuedCount}
        pressureState={overview?.pressure_state}
        refreshing={refreshing}
        closingAll={closingAll}
        hasManagedResources={hasManagedResources}
        controlsDisabled={managementMutationBusy}
        showLifecycleControls={activeTab === 'lifecycle'}
        canCloseAll={canCloseAll}
        closeAllLabel={installationWideCloseCopy.button}
        onRefresh={() => void refreshAll()}
        onCloseAll={handleCloseAll}
      />

      <Tabs
        activeTab={activeTab}
        onChange={handleTabChange}
        type='line'
        lazyload
        destroyOnHide
        className='flex flex-col flex-1 min-h-0 [&>.arco-tabs-header]:shrink-0 [&>.arco-tabs-content]:flex-1 [&>.arco-tabs-content]:min-h-0 [&>.arco-tabs-content]:overflow-hidden [&>.arco-tabs-content]:pt-8px [&>.arco-tabs-content>.arco-tabs-content-inner]:h-full [&>.arco-tabs-content>.arco-tabs-content-inner]:min-h-0 [&_.arco-tabs-pane]:h-full [&_.arco-tabs-pane]:min-h-0'
      >
        <Tabs.TabPane key='lifecycle' title={t('browser.tabs.lifecycle')}>
          <div className='size-full min-h-0 flex flex-col'>
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

            {overview && !capabilityUnavailable && <BrowserHostDiagnostics overview={overview} />}

            {capabilityUnavailable ? (
              <div className='flex-1 min-h-0 flex items-center justify-center bg-1 rd-12px border border-solid border-[var(--color-border-2)]'>
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
              <div className='flex-1 min-h-0 flex items-center justify-center bg-1 rd-12px border border-solid border-[var(--color-border-2)] p-20px'>
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
              <div className='flex-1 min-h-0 flex items-center justify-center bg-1 rd-12px border border-solid border-[var(--color-border-2)]'>
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
        </Tabs.TabPane>
        <Tabs.TabPane key='settings' title={t('browser.tabs.settings')}>
          <div className='size-full min-h-0 max-w-1024px mx-auto overflow-hidden'>
            <BrowserUseSettingsContent />
          </div>
        </Tabs.TabPane>
      </Tabs>
    </div>
  );
};

export default BrowserPage;
