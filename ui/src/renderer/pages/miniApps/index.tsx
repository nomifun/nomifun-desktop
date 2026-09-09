/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  MiniAppLibraryResponse,
  MiniAppSummary,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Button, Input, Spin } from '@arco-design/web-react';
import {
  AddOne,
  ApplicationOne,
  Download,
  ImportAndExport,
  Refresh,
  Right,
  Search,
} from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import MiniAppCreateProjectDialog from './MiniAppCreateProjectDialog';
import MiniAppTransferDialog from './MiniAppTransferDialog';
import {
  formatMiniAppTimestamp,
  miniAppReleaseStage,
} from './model';
import {
  MiniAppKindBadge,
  MiniAppLifecycleBadge,
  MiniAppStatePanel,
  StatusBadge,
} from './MiniAppM1State';
import styles from './MiniAppWorkbench.module.css';

const formatError = (error: unknown): string => {
  if (isBackendHttpError(error)) {
    return error.code
      ? `${error.code}: ${error.backendMessage || error.message}`
      : error.backendMessage || error.message;
  }
  return error instanceof Error ? error.message : String(error);
};

const releaseCount = (app: MiniAppSummary): number =>
  [app.releases.ready, app.releases.active, app.releases.previous].filter(Boolean).length;

export const MiniAppLibraryCard: React.FC<{
  app: MiniAppSummary;
  locale: string;
  onOpen: (app: MiniAppSummary) => void;
}> = ({ app, locale, onOpen }) => {
  const { t } = useTranslation();
  const stage = miniAppReleaseStage(app);
  const stageKey = `miniApps.library.releaseStage.${stage}` as const;
  const titleId = `miniapp-library-card-title-${app.miniapp_id}`;
  const descriptionId = `miniapp-library-card-description-${app.miniapp_id}`;

  return (
    <article
      className={styles.libraryCard}
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
    >
      <div className={styles.libraryCardHeader}>
        <span className={styles.libraryCardIcon} aria-hidden='true'>
          <ApplicationOne theme='outline' size='20' />
        </span>
        <div className={styles.libraryCardCopy}>
          <h2 id={titleId} className={styles.libraryCardTitle}>
            {app.display_name}
          </h2>
          <div className={styles.libraryBadgeRow}>
            <MiniAppKindBadge kind={app.kind} />
            <MiniAppLifecycleBadge lifecycle={app.lifecycle} />
          </div>
        </div>
      </div>

      <p id={descriptionId} className={styles.libraryCardDescription}>
        {app.description || t('miniApps.library.noDescription')}
      </p>

      <div className={styles.libraryBadgeRow}>
        <StatusBadge
          label={t(stageKey)}
          tone={stage === 'active' ? 'success' : stage === 'ready' ? 'info' : 'muted'}
        />
        <StatusBadge
          label={
            app.surface_available
              ? t('miniApps.library.surface.available')
              : t('miniApps.library.surface.unavailable')
          }
          tone={app.surface_available ? 'success' : 'muted'}
        />
      </div>

      <div className={styles.libraryFacts}>
        <div className={styles.fact}>
          <span className={styles.factLabel}>{t('miniApps.library.facts.releases')}</span>
          <span className={styles.factValue}>{releaseCount(app)}</span>
        </div>
        <div className={styles.fact}>
          <span className={styles.factLabel}>{t('miniApps.library.facts.pointerRevision')}</span>
          <span className={styles.factValue}>{app.releases.pointer_revision}</span>
        </div>
        <div className={styles.fact}>
          <span className={styles.factLabel}>{t('miniApps.library.facts.activeEpoch')}</span>
          <span className={styles.factValue}>{app.releases.active_release_epoch}</span>
        </div>
        <div className={styles.fact}>
          <span className={styles.factLabel}>{t('miniApps.library.facts.productRevision')}</span>
          <span className={styles.factValue}>{app.product_revision}</span>
        </div>
      </div>

      <div className={styles.libraryCardFooter}>
        <span className={styles.updatedAt}>
          {t('miniApps.library.updatedAt', {
            time: formatMiniAppTimestamp(app.updated_at_ms, locale),
          })}
        </span>
        <Button
          size='small'
          icon={<Right theme='outline' size='13' />}
          aria-label={`${t('miniApps.library.openWorkshop')}: ${app.display_name}`}
          onClick={() => onOpen(app)}
        >
          {t('miniApps.library.openWorkshop')}
        </Button>
      </div>
    </article>
  );
};

const MiniAppsListPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const [message, messageContext] = useArcoMessage({ maxCount: 4 });
  const [library, setLibrary] = useState<MiniAppLibraryResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [createVisible, setCreateVisible] = useState(false);
  const [transferMode, setTransferMode] = useState<
    'import_share' | 'import_artifact' | 'import_backup' | null
  >(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const next = await ipcBridge.miniapps.library.invoke();
      setLibrary(next);
      setFailure(null);
    } catch (error) {
      console.error('[miniapps] failed to load M1 Library', error);
      setFailure(formatError(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const filteredApps = useMemo(() => {
    const apps = library?.miniapps ?? [];
    const query = searchQuery.trim().toLocaleLowerCase();
    if (!query) return apps;
    return apps.filter((app) =>
      [app.display_name, app.description ?? '', app.kind, app.lifecycle].some((value) =>
        value.toLocaleLowerCase().includes(query)
      )
    );
  }, [library, searchQuery]);

  const openWorkshop = useCallback(
    (app: MiniAppSummary) => navigate(`/mini-apps/${app.miniapp_id}`),
    [navigate]
  );

  const handleCreated = useCallback(
    (workshop: MiniAppWorkshop) => {
      setCreateVisible(false);
      message.success(t('miniApps.create.success'));
      navigate(`/mini-apps/${workshop.miniapp.miniapp_id}`);
    },
    [message, navigate, t]
  );

  const handleImported = useCallback(
    (workshop: MiniAppWorkshop) => {
      const completedMode = transferMode;
      setTransferMode(null);
      message.success(
        t(
          completedMode === 'import_artifact'
            ? 'miniApps.messages.artifactImported'
            : completedMode === 'import_backup'
              ? 'miniApps.messages.backupImported'
              : 'miniApps.messages.shareImported'
        )
      );
      navigate(`/mini-apps/${workshop.miniapp.miniapp_id}`);
    },
    [message, navigate, t, transferMode]
  );

  const apps = library?.miniapps ?? [];

  return (
    <>
      {messageContext}
      <HubPageShell
        title={t('miniApps.title')}
        subtitle={t('miniApps.subtitle')}
        className={styles.page}
        maxWidthClass='md:max-w-1200px'
        toolbar={
          <div className={styles.toolbar}>
            <div className={styles.toolbarMeta}>
              <span>{t('miniApps.library.revision', { revision: library?.library_revision ?? '-' })}</span>
              {!loading && <span>{t('miniApps.library.count', { count: apps.length })}</span>}
            </div>
            <div className='flex items-center gap-8px'>
              <Button
                size='small'
                icon={<ImportAndExport theme='outline' size='14' />}
                disabled={loading || Boolean(failure)}
                onClick={() => setTransferMode('import_share')}
              >
                {t('miniApps.transfer.importShare.submit')}
              </Button>
              <Button
                size='small'
                icon={<Download theme='outline' size='14' />}
                disabled={loading || Boolean(failure)}
                onClick={() => setTransferMode('import_artifact')}
              >
                {t('miniApps.transfer.importArtifact.submit')}
              </Button>
              <Button
                size='small'
                icon={<Download theme='outline' size='14' />}
                disabled={loading || Boolean(failure)}
                onClick={() => setTransferMode('import_backup')}
              >
                {t('miniApps.transfer.importBackup.submit')}
              </Button>
              <Button
                size='small'
                icon={<Refresh theme='outline' size='14' />}
                loading={loading}
                onClick={() => void refresh()}
              >
                {t('miniApps.actions.refresh')}
              </Button>
              <Button
                type='primary'
                icon={<AddOne theme='outline' size='14' />}
                onClick={() => setCreateVisible(true)}
              >
                {t('miniApps.create.action')}
              </Button>
            </div>
          </div>
        }
      >
        {failure ? (
          <MiniAppStatePanel
            title={t('miniApps.errors.loadLibraryTitle')}
            body={failure}
            onRetry={() => void refresh()}
          />
        ) : loading ? (
          <div className={styles.statePanel}>
            <Spin size={26} />
            <span className={styles.stateBody}>{t('miniApps.states.loadingLibrary')}</span>
          </div>
        ) : (
          <section className={styles.libraryPanel} aria-labelledby='miniapp-library-title'>
            <div className={styles.libraryHeader}>
              <div className={styles.libraryHeading}>
                <h2 id='miniapp-library-title' className={styles.libraryTitle}>
                  {t('miniApps.library.heading')}
                </h2>
                <p className={styles.libraryHint}>{t('miniApps.library.hint')}</p>
              </div>
              {apps.length > 0 && (
                <div className={styles.librarySearch}>
                  <Input
                    prefix={<Search theme='outline' size='14' />}
                    value={searchQuery}
                    allowClear
                    placeholder={t('miniApps.library.searchPlaceholder')}
                    onChange={setSearchQuery}
                  />
                </div>
              )}
            </div>

            {apps.length === 0 ? (
              <div className={styles.emptyState}>
                <span className={styles.stateIcon} aria-hidden='true'>
                  <ApplicationOne theme='outline' size='24' />
                </span>
                <span className={styles.stateTitle}>{t('miniApps.library.emptyTitle')}</span>
                <span className={styles.stateBody}>{t('miniApps.library.emptyBody')}</span>
                <Button
                  type='primary'
                  icon={<AddOne theme='outline' size='14' />}
                  onClick={() => setCreateVisible(true)}
                >
                  {t('miniApps.create.action')}
                </Button>
              </div>
            ) : filteredApps.length === 0 ? (
              <div className={styles.emptyState}>
                <span className={styles.stateIcon} aria-hidden='true'>
                  <Search theme='outline' size='22' />
                </span>
                <span className={styles.stateTitle}>{t('miniApps.library.noMatchTitle')}</span>
                <span className={styles.stateBody}>{t('miniApps.library.noMatchBody')}</span>
              </div>
            ) : (
              <div className={styles.libraryGrid}>
                {filteredApps.map((app) => (
                  <MiniAppLibraryCard
                    key={app.miniapp_id}
                    app={app}
                    locale={i18n.language}
                    onOpen={openWorkshop}
                  />
                ))}
              </div>
            )}
          </section>
        )}
      </HubPageShell>
      <MiniAppCreateProjectDialog
        visible={createVisible}
        libraryRevision={library?.library_revision ?? 0}
        onCancel={() => setCreateVisible(false)}
        onCreated={handleCreated}
      />
      <MiniAppTransferDialog
        mode={transferMode ?? 'import_share'}
        visible={transferMode !== null}
        libraryRevision={library?.library_revision ?? 0}
        onCancel={() => setTransferMode(null)}
        onImported={handleImported}
        onExported={() => undefined}
      />
    </>
  );
};

export default MiniAppsListPage;
