/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PluginConsumerSurface,
  PluginDetail,
  PluginMountId,
  PluginSummary,
  PluginTargetRef,
} from '@/common/types/pluginPlatform';
import { Button } from '@arco-design/web-react';
import {
  Delete,
  PauseOne,
  PlayOne,
  Plug,
  Redo,
  Refresh,
} from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import {
  formatPluginTimestamp,
  pluginMountActions,
  shortPluginIdentity,
} from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';
import {
  PluginLifecycleBadge,
  PluginStatePanel,
  StatusBadge,
} from './PluginWorkbenchState';

export type PluginMountBusyAction =
  | 'enable'
  | 'disable'
  | 'retry'
  | 'restore'
  | 'uninstall'
  | 'delete_data'
  | null;

interface PluginLibraryViewProps {
  plugins: PluginSummary[];
  selectedMountId: PluginMountId | null;
  detail?: PluginDetail;
  detailLoading: boolean;
  detailFailure: PluginLoadFailure | null;
  mutationFailure: PluginLoadFailure | null;
  busyAction: PluginMountBusyAction;
  locale: string;
  onSelect: (mountId: PluginMountId) => void;
  onRetryDetail: () => void;
  onEnable: () => void;
  onDisable: () => void;
  onRetryMount: () => void;
  onRestore: () => void;
  onUninstall: () => void;
  onDeleteData: () => void;
}

const TargetColumn: React.FC<{
  label: string;
  target?: PluginTargetRef;
  empty: string;
}> = ({ label, target, empty }) => (
  <div className={styles.targetColumn}>
    <span className={styles.targetLabel}>{label}</span>
    {target ? (
      <>
        <span className={styles.targetVersion}>v{target.package_version}</span>
        <span className={styles.targetPackage} title={target.package_id}>
          {target.package_id}
        </span>
        <span className={`${styles.targetDigest} ${styles.mono}`} title={target.artifact_digest}>
          {shortPluginIdentity(target.artifact_digest)}
        </span>
      </>
    ) : (
      <span className={styles.targetPackage}>{empty}</span>
    )}
  </div>
);

const consumerLabel = (
  surface: PluginConsumerSurface,
  t: ReturnType<typeof useTranslation>['t']
): string => {
  const labels: Record<PluginConsumerSurface, string> = {
    agent: t('pluginWorkbench.consumer.agent'),
    gateway: t('pluginWorkbench.consumer.gateway'),
    knowledge: t('pluginWorkbench.consumer.knowledge'),
    remote: t('pluginWorkbench.consumer.remote'),
    automation: t('pluginWorkbench.consumer.automation'),
    ui: t('pluginWorkbench.consumer.ui'),
    miniapp_service: t('pluginWorkbench.consumer.miniappService'),
  };
  return labels[surface];
};

const PluginMountDetail: React.FC<
  Omit<
    PluginLibraryViewProps,
    'plugins' | 'selectedMountId' | 'detailLoading' | 'detailFailure' | 'onSelect'
  > & { detail: PluginDetail }
> = ({
  detail,
  mutationFailure,
  busyAction,
  locale,
  onEnable,
  onDisable,
  onRetryMount,
  onRestore,
  onUninstall,
  onDeleteData,
}) => {
  const { t } = useTranslation();
  const actions = pluginMountActions(detail);
  const disabled = busyAction !== null;
  const enabled = detail.summary.lifecycle === 'enabled';

  return (
    <>
      <header className={styles.detailHeader}>
        <div className={styles.detailHeaderCopy}>
          <span className={styles.eyebrow}>{t('pluginWorkbench.library.detailEyebrow')}</span>
          <div className={styles.detailTitleRow}>
            <h2 className={styles.detailTitle}>{detail.summary.display_name}</h2>
            <PluginLifecycleBadge lifecycle={detail.summary.lifecycle} />
          </div>
          {detail.summary.description && (
            <p className={styles.detailDescription}>{detail.summary.description}</p>
          )}
        </div>
      </header>

      {mutationFailure && (
        <div className={`${styles.notice} ${styles.noticeError}`}>
          <span>{t('pluginWorkbench.states.mutationErrorTitle')}</span>
          <span>{mutationFailure.message}</span>
        </div>
      )}

      <div className={styles.actionBar}>
        {actions.canToggleEnabled && (
          <Button
            type={enabled ? 'secondary' : 'primary'}
            icon={
              enabled ? (
                <PauseOne theme='outline' size='14' />
              ) : (
                <PlayOne theme='outline' size='14' />
              )
            }
            loading={busyAction === (enabled ? 'disable' : 'enable')}
            disabled={disabled}
            onClick={enabled ? onDisable : onEnable}
          >
            {enabled
              ? t('pluginWorkbench.actions.disable')
              : t('pluginWorkbench.actions.enable')}
          </Button>
        )}
        {actions.canRetry && (
          <Button
            icon={<Refresh theme='outline' size='14' />}
            loading={busyAction === 'retry'}
            disabled={disabled}
            onClick={onRetryMount}
          >
            {t('pluginWorkbench.actions.retryRuntime')}
          </Button>
        )}
        {actions.canRestore && (
          <Button
            icon={<Redo theme='outline' size='14' />}
            loading={busyAction === 'restore'}
            disabled={disabled}
            onClick={onRestore}
          >
            {t('pluginWorkbench.actions.restore')}
          </Button>
        )}
        {actions.canUninstall && (
          <Button
            status='danger'
            icon={<Delete theme='outline' size='14' />}
            loading={busyAction === 'uninstall'}
            disabled={disabled}
            onClick={onUninstall}
          >
            {t('pluginWorkbench.actions.uninstall')}
          </Button>
        )}
        {actions.canDeleteData && (
          <Button
            status='danger'
            icon={<Delete theme='outline' size='14' />}
            loading={busyAction === 'delete_data'}
            disabled={disabled}
            onClick={onDeleteData}
          >
            {t('pluginWorkbench.actions.deleteData')}
          </Button>
        )}
      </div>

      {detail.summary.lifecycle === 'uninstalled_data_retained' && (
        <div className={`${styles.notice} ${styles.noticeWarning}`}>
          {t('pluginWorkbench.library.retainedDataNotice')}
        </div>
      )}

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.detail.identity')}</h3>
            <p className={styles.sectionHint}>{t('pluginWorkbench.detail.identityHint')}</p>
          </div>
        </div>
        <div className={styles.factGrid}>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.mountId')}</span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={detail.summary.mount_id}
            >
              {shortPluginIdentity(detail.summary.mount_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.revision')}</span>
            <span className={styles.factValue}>{detail.summary.mount_revision}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.updatedAt')}</span>
            <span className={styles.factValue}>
              {formatPluginTimestamp(detail.summary.updated_at_ms, locale)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.linkedProject')}</span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={detail.summary.linked_project_id}
            >
              {shortPluginIdentity(detail.summary.linked_project_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.config')}</span>
            <span className={styles.factValue}>
              {detail.config.valid
                ? t('pluginWorkbench.detail.configValid')
                : t('pluginWorkbench.detail.configInvalid')}
              {' · '}
              r{detail.config.config_revision}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.credentials')}</span>
            <span className={styles.factValue}>
              {t('pluginWorkbench.detail.credentialCount', {
                count: detail.credential_slots.length,
              })}
              {' · '}
              r{detail.credential_bindings_revision}
            </span>
          </div>
        </div>
        {detail.last_error_code && (
          <div className={`${styles.notice} ${styles.noticeError}`} style={{ margin: '12px 0 0' }}>
            {t('pluginWorkbench.detail.lastError', { code: detail.last_error_code })}
          </div>
        )}
      </section>

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.detail.targets')}</h3>
            <p className={styles.sectionHint}>{t('pluginWorkbench.detail.targetsHint')}</p>
          </div>
        </div>
        <div className={styles.targetGrid}>
          <TargetColumn
            label={t('pluginWorkbench.detail.currentTarget')}
            target={detail.summary.current}
            empty={t('pluginWorkbench.common.none')}
          />
          <TargetColumn
            label={t('pluginWorkbench.detail.previousTarget')}
            target={detail.summary.previous}
            empty={t('pluginWorkbench.common.none')}
          />
        </div>
      </section>

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.detail.capabilities')}</h3>
            <p className={styles.sectionHint}>
              {t('pluginWorkbench.detail.capabilityCount', {
                count: detail.capabilities.length,
              })}
            </p>
          </div>
        </div>
        {detail.capabilities.length === 0 ? (
          <PluginStatePanel
            compact
            title={t('pluginWorkbench.detail.noCapabilities')}
            body={t('pluginWorkbench.detail.noCapabilitiesBody')}
          />
        ) : (
          <div className={styles.capabilityList}>
            {detail.capabilities.map((capability) => (
              <div
                key={capability.provenance.contribution_id}
                className={styles.capabilityRow}
              >
                <div className={styles.capabilityHeader}>
                  <span className={styles.capabilityName}>{capability.display_name}</span>
                  <StatusBadge
                    label={`v${capability.capability_version}`}
                    tone='info'
                  />
                </div>
                <div className={`${styles.listMeta} ${styles.mono}`} title={capability.capability_id}>
                  {capability.capability_id}
                </div>
                {capability.description && (
                  <div className={styles.capabilityDescription}>
                    {capability.description}
                  </div>
                )}
                <div className={styles.chipRow}>
                  {capability.consumer_availability.map((availability) => (
                    <span
                      key={`${availability.surface}:${availability.status}`}
                      className={styles.chip}
                      title={availability.reason_code}
                    >
                      {consumerLabel(availability.surface, t)}
                      {' · '}
                      {availability.status}
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
};

const PluginLibraryView: React.FC<PluginLibraryViewProps> = (props) => {
  const { t } = useTranslation();
  const {
    plugins,
    selectedMountId,
    detail,
    detailLoading,
    detailFailure,
    onSelect,
    onRetryDetail,
  } = props;

  return (
    <div className={styles.workspace}>
      <aside className={styles.collection} aria-label={t('pluginWorkbench.library.ariaLabel')}>
        <div className={styles.collectionHeader}>
          <span className={styles.collectionTitle}>{t('pluginWorkbench.library.title')}</span>
          <span className={styles.collectionCount}>
            {t('pluginWorkbench.common.count', { count: plugins.length })}
          </span>
        </div>
        {plugins.length === 0 ? (
          <PluginStatePanel
            compact
            title={t('pluginWorkbench.library.emptyTitle')}
            body={t('pluginWorkbench.library.emptyBody')}
          />
        ) : (
          <div className={styles.list}>
            {plugins.map((plugin) => (
              <button
                key={plugin.mount_id}
                type='button'
                className={`${styles.listRow} ${
                  selectedMountId === plugin.mount_id ? styles.listRowActive : ''
                }`}
                onClick={() => onSelect(plugin.mount_id)}
              >
                <span className={styles.listIcon}>
                  <Plug theme='outline' size='17' />
                </span>
                <span className={styles.listCopy}>
                  <span className={styles.listName}>{plugin.display_name}</span>
                  <span className={styles.listMeta}>
                    {plugin.current?.package_version
                      ? `v${plugin.current.package_version} · `
                      : ''}
                    {t('pluginWorkbench.library.contributionCount', {
                      count: plugin.contribution_count,
                    })}
                  </span>
                </span>
                <PluginLifecycleBadge lifecycle={plugin.lifecycle} />
              </button>
            ))}
          </div>
        )}
      </aside>

      <main className={styles.detail}>
        {!selectedMountId ? (
          <PluginStatePanel
            title={t('pluginWorkbench.library.selectTitle')}
            body={t('pluginWorkbench.library.selectBody')}
          />
        ) : detailLoading && !detail ? (
          <PluginStatePanel loading body={t('pluginWorkbench.states.loadingMount')} />
        ) : detailFailure ? (
          <PluginStatePanel failure={detailFailure} onRetry={onRetryDetail} />
        ) : detail ? (
          <PluginMountDetail {...props} detail={detail} />
        ) : (
          <PluginStatePanel
            title={t('pluginWorkbench.library.selectTitle')}
            body={t('pluginWorkbench.library.selectBody')}
          />
        )}
      </main>
    </div>
  );
};

export default PluginLibraryView;
