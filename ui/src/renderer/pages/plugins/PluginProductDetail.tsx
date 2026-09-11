import type { PluginDetail } from '@/common/types/pluginPlatform';
import { Alert, Button, Switch, Tag } from '@arco-design/web-react';
import {
  ArrowLeft,
  Code,
  Delete,
  Link,
  PlayOne,
  Redo,
  SettingTwo,
  User,
} from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginMountBusyAction } from './PluginLibraryView';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import { pluginMountActions, shortPluginIdentity } from './pluginWorkbenchModel';
import styles from './PluginProductSurface.module.css';

interface PluginProductDetailProps {
  detail: PluginDetail;
  agentUsage: Array<{ presetId: string; displayName: string; capabilityCount: number }>;
  failure: PluginLoadFailure | null;
  busyAction: PluginMountBusyAction;
  onBack: () => void;
  onContinueCreating: () => void;
  onOpenAgent: (capabilityId?: string, presetId?: string) => void;
  onConfigure: () => void;
  onEnable: () => void;
  onDisable: () => void;
  onRetry: () => void;
  onRestore: () => void;
  onUninstall: () => void;
  onDeleteData: () => void;
}

const PluginProductDetail: React.FC<PluginProductDetailProps> = ({
  detail,
  agentUsage,
  failure,
  busyAction,
  onBack,
  onContinueCreating,
  onOpenAgent,
  onConfigure,
  onEnable,
  onDisable,
  onRetry,
  onRestore,
  onUninstall,
  onDeleteData,
}) => {
  const { t } = useTranslation();
  const actions = pluginMountActions(detail);
  const enabled = detail.summary.lifecycle === 'enabled';
  const busy = busyAction !== null;
  const agentCapabilities = detail.capabilities.filter((capability) =>
    capability.consumer_availability.some((consumer) => consumer.surface === 'agent')
  );

  return (
    <div>
      <Button type='text' icon={<ArrowLeft size={16} />} onClick={onBack}>
        {t('pluginWorkbench.product.backToPlugins')}
      </Button>
      <header className={styles.detailHeader}>
        <div className={styles.detailIdentity}>
          <span className={styles.pluginIcon}><Link theme='outline' size={20} /></span>
          <div>
            <span className={styles.eyebrow}>{t('pluginWorkbench.product.capabilityExtension')}</span>
            <h2>{detail.summary.display_name}</h2>
            <p>{detail.summary.description || t('pluginWorkbench.product.noDescription')}</p>
          </div>
        </div>
        {actions.canToggleEnabled && (
          <Switch
            checked={enabled}
            loading={busyAction === (enabled ? 'disable' : 'enable')}
            checkedText={t('pluginWorkbench.lifecycle.enabled')}
            uncheckedText={t('pluginWorkbench.lifecycle.disabled')}
            onChange={(next: boolean) => next ? onEnable() : onDisable()}
          />
        )}
      </header>

      {failure && <Alert type='error' showIcon content={failure.message} />}
      {detail.last_error_code && (
        <Alert
          type='warning'
          showIcon
          content={t('pluginWorkbench.product.runtimeIssue', { code: detail.last_error_code })}
          action={actions.canRetry ? <Button size='small' onClick={onRetry}>{t('pluginWorkbench.actions.retry')}</Button> : undefined}
        />
      )}

      <div className={styles.detailActions}>
        <Button type='primary' icon={<User size={15} />} disabled={!agentCapabilities.length} onClick={() => onOpenAgent(agentCapabilities[0]?.capability_id)}>
          {t('pluginWorkbench.product.addToAgent')}
        </Button>
        {detail.summary.linked_project_id && (
          <Button icon={<Code size={15} />} onClick={onContinueCreating}>
            {t('pluginWorkbench.product.continueWithAi')}
          </Button>
        )}
        <Button icon={<SettingTwo size={15} />} disabled={busy || !detail.summary.current} onClick={onConfigure}>
          {t('pluginWorkbench.product.usageSettings')}
        </Button>
        {actions.canRestore && (
          <Button icon={<Redo size={15} />} disabled={busy} onClick={onRestore}>
            {t('pluginWorkbench.actions.restore')}
          </Button>
        )}
      </div>

      <div className={styles.detailGrid}>
        <section className={styles.detailSection}>
          <h2>{t('pluginWorkbench.product.enhancements')}</h2>
          <p>{t('pluginWorkbench.product.enhancementsHint')}</p>
          <div className={styles.capabilityList}>
            {detail.capabilities.map((capability) => (
              <div key={capability.capability_id} className={styles.capabilityRow}>
                <span className={styles.capabilityIcon}><Link theme='outline' size={15} /></span>
                <div>
                  <strong>{capability.display_name}</strong>
                  <small>{capability.description || capability.capability_id}</small>
                </div>
              </div>
            ))}
            {!detail.capabilities.length && <p>{t('pluginWorkbench.detail.noCapabilitiesBody')}</p>}
          </div>
        </section>

        <section className={styles.agentLinkSection}>
          <div className={styles.agentLinkHeader}>
            <div>
              <h2>{t('pluginWorkbench.product.agentLinkTitle')}</h2>
              <p>{t('pluginWorkbench.product.agentLinkHint')}</p>
            </div>
            <Button size='small' disabled={!agentCapabilities.length} onClick={() => onOpenAgent(agentCapabilities[0]?.capability_id, agentUsage[0]?.presetId)}>
              {t('pluginWorkbench.product.manageInAgent')}
            </Button>
          </div>
          <div className={styles.agentLinkList}>
            {agentUsage.map((usage) => (
              <div key={usage.presetId} className={styles.agentLinkRow}>
                <span>
                  <strong>{usage.displayName}</strong>
                  <small>{t('pluginWorkbench.product.agentUsingCount', { count: usage.capabilityCount })}</small>
                </span>
                <Tag color='green'>{t('pluginWorkbench.product.inUse')}</Tag>
              </div>
            ))}
            {!agentUsage.length && <p>{agentCapabilities.length ? t('pluginWorkbench.product.noAgentUsage') : t('pluginWorkbench.product.noAgentCapabilities')}</p>}
          </div>
        </section>
      </div>

      <details className={styles.technicalDetails}>
        <summary>{t('common.technical_details')}</summary>
        <div className={styles.technicalFacts}>
          <div><span>{t('pluginWorkbench.detail.mountId')}</span><code>{shortPluginIdentity(detail.summary.mount_id)}</code></div>
          <div><span>{t('pluginWorkbench.detail.currentTarget')}</span><code>{detail.summary.current?.package_version ?? '—'}</code></div>
          <div><span>{t('pluginWorkbench.detail.previousTarget')}</span><code>{detail.summary.previous?.package_version ?? '—'}</code></div>
          <div><span>{t('pluginWorkbench.detail.revision')}</span><code>{detail.summary.mount_revision}</code></div>
          <div><span>{t('pluginWorkbench.detail.config')}</span><code>r{detail.config.config_revision}</code></div>
          <div><span>{t('pluginWorkbench.detail.credentials')}</span><code>{detail.credential_slots.length}</code></div>
        </div>
        <div className={styles.advancedActions}>
          {actions.canUninstall && <Button status='danger' icon={<Delete size={14} />} disabled={busy} onClick={onUninstall}>{t('pluginWorkbench.actions.uninstall')}</Button>}
          {actions.canDeleteData && <Button status='danger' icon={<Delete size={14} />} disabled={busy} onClick={onDeleteData}>{t('pluginWorkbench.actions.deleteData')}</Button>}
          {actions.canRetry && <Button icon={<PlayOne size={14} />} disabled={busy} onClick={onRetry}>{t('pluginWorkbench.actions.retryRuntime')}</Button>}
        </div>
      </details>
    </div>
  );
};

export default PluginProductDetail;
