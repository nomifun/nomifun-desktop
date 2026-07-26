/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Alert, Button, Empty, Message, Modal, Spin } from '@arco-design/web-react';
import { WebPage } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useSearchParams } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import {
  migrateBrowserDisplayMode,
  type BrowserDisplayMode,
} from '@/common/browser/browserSettings';
import {
  resolveBrowserOverviewCapabilities,
  type IBrowserLane,
} from '@/common/browser/browserTypes';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { useConversationHistoryContext } from '@/renderer/hooks/context/ConversationHistoryContext';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import BrowserInventoryTree from './BrowserInventoryTree';
import BrowserLaneDetails from './BrowserLaneDetails';
import BrowserPageHeader from './BrowserPageHeader';
import {
  browserConversationSearchParamsForLane,
  browserLaneCounts,
  buildBrowserInventoryTree,
  pickDefaultBrowserLaneId,
  resolveBrowserConversationId,
  type BrowserConversationGroup,
} from './browserInventoryModel';
import {
  browserInstallationWideCloseCopy,
  requestBrowserCloseAll,
  requestBrowserConversationClose,
  requestBrowserLaneClose,
  runBrowserCloseAll,
  runBrowserConversationClose,
  runBrowserLaneClose,
  type BrowserConfirmationRequest,
} from './browserManagementActions';
import { useBrowserInventory } from './useBrowserInventory';
import BrowserHostDiagnostics from './BrowserHostDiagnostics';

const BrowserPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const location = useLocation();
  const layout = useLayoutContext();
  const { conversations } = useConversationHistoryContext();
  const { lanes, overview, loading, refreshing, error, refresh } = useBrowserInventory();
  const [storedDisplayMode] = useConfig('agent.browserUse.displayMode');
  const [storedLegacySilent] = useConfig('agent.browserUse.silent');
  const [displayModeReady, setDisplayModeReady] = useState(configService.isInitialized());
  const [selectedLaneId, setSelectedLaneId] = useState<string | null>(null);
  const [busyLaneId, setBusyLaneId] = useState<string | null>(null);
  const [busyConversationId, setBusyConversationId] = useState<string | null>(null);
  const [closingAll, setClosingAll] = useState(false);

  useEffect(() => {
    let active = true;
    void configService.whenReady().finally(() => {
      if (active) setDisplayModeReady(true);
    });
    return () => {
      active = false;
    };
  }, []);

  const displayMode: BrowserDisplayMode | null = displayModeReady
    ? migrateBrowserDisplayMode({
        displayMode: storedDisplayMode,
        silent: storedLegacySilent,
      }).displayMode
    : null;
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
  const { canCloseAll } = resolveBrowserOverviewCapabilities(overview);

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
      }),
    [refresh, t]
  );

  const handleCloseLane = useCallback(
    (lane: IBrowserLane) => {
      void requestBrowserLaneClose(lane, closeLane, confirmDanger, {
        title: t('browser.close.laneActiveTitle'),
        content: t('browser.close.laneActiveContent'),
        okText: t('browser.close.laneAction'),
        cancelText: t('browser.close.keepOpen'),
      });
    },
    [closeLane, confirmDanger, t]
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
      }),
    [refresh, t]
  );

  const handleCloseConversation = useCallback(
    (group: BrowserConversationGroup) => {
      requestBrowserConversationClose(group, closeConversation, confirmDanger, {
        title: t('browser.close.conversationTitle'),
        content: t('browser.close.conversationContent', { count: group.lanes.length }),
        okText: t('browser.close.conversationAction'),
        cancelText: t('browser.close.keepOpen'),
      });
    },
    [closeConversation, confirmDanger, t]
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
      }),
    [installationWideCloseCopy.success, refresh]
  );

  const handleCloseAll = useCallback(() => {
    requestBrowserCloseAll(closeAll, confirmDanger, {
      title: installationWideCloseCopy.title,
      content: `${installationWideCloseCopy.warning} ${t('browser.close.allContent')}`,
      okText: installationWideCloseCopy.action,
      cancelText: t('browser.close.cancel'),
    });
  }, [closeAll, confirmDanger, installationWideCloseCopy, t]);

  const capabilityUnavailable = overview?.supported === false || overview?.enabled === false;

  return (
    <div className='size-full min-h-0 flex flex-col p-16px box-border bg-bg-2'>
      <BrowserPageHeader
        runningCount={runningCount}
        queuedCount={queuedCount}
        pressureState={overview?.pressure_state}
        refreshing={refreshing}
        closingAll={closingAll}
        hasLanes={lanes.length > 0}
        canCloseAll={canCloseAll}
        closeAllLabel={installationWideCloseCopy.button}
        onRefresh={() => void refresh()}
        onCloseAll={handleCloseAll}
      />

      {error && (
        <Alert
          type='warning'
          showIcon
          className='mb-12px shrink-0'
          content={t('browser.page.inventoryUnavailable', { error })}
          action={
            <Button size='mini' onClick={() => void refresh()}>
              {t('browser.page.retry')}
            </Button>
          }
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
                ? 'shrink-0'
                : 'min-h-0 overflow-y-auto pr-2px'
            }
            aria-label={t('browser.page.inventoryAria')}
          >
            <BrowserInventoryTree
              groups={groups}
              selectedLaneId={selectedLaneId}
              currentConversationId={currentConversationId}
              busyLaneId={busyLaneId}
              busyConversationId={busyConversationId}
              onSelectLane={handleSelectLane}
              onCloseLane={handleCloseLane}
              onCloseConversation={handleCloseConversation}
            />
          </aside>
          <main className={layout?.isMobile ? 'min-h-0' : 'min-h-0 overflow-y-auto pr-2px'}>
            {selectedLane ? (
              <BrowserLaneDetails
                lane={selectedLane}
                displayMode={displayMode}
                closing={busyLaneId === selectedLane.lane_id}
                onClose={handleCloseLane}
                onInventoryRefresh={refresh}
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
