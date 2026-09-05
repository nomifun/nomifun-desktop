import type {
  InstallationTokenStateResponse,
  ResolveAgentPresetPreviewResponse,
} from '@/common/types/agentPlatform';
import { Alert, Collapse, Tag } from '@arco-design/web-react';
import { CheckOne, CloseOne, Connection, Info, Terminal } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { previewDiagnosticMessage } from './model';
import styles from './AgentSettingsPage.module.css';

type PreviewInspectorProps = {
  preview: ResolveAgentPresetPreviewResponse | null;
  tokenState: InstallationTokenStateResponse | null;
};

const PreviewInspector: React.FC<PreviewInspectorProps> = ({ preview, tokenState }) => {
  const { t } = useTranslation();

  if (!preview) {
    return (
      <section className={styles.previewEmpty}>
        <Info theme='outline' size='18' />
        <div>
          <strong>{t('agentSettings.preview.emptyTitle')}</strong>
          <span>{t('agentSettings.preview.emptyBody')}</span>
        </div>
      </section>
    );
  }

  const metrics = [
    ['initial_count', preview.summary.initial_count],
    ['on_demand_count', preview.summary.on_demand_count],
    ['active_at_start_count', preview.summary.active_at_start_count],
    ['model_tool_count', preview.summary.model_tool_count],
    ['context_contributor_count', preview.summary.context_contributor_count],
    ['skill_count', preview.summary.skill_count],
    ['mcp_count', preview.summary.mcp_count],
    ['resource_binding_count', preview.summary.resource_binding_count],
  ] as const;

  return (
    <div className={styles.previewStack}>
      <div className={styles.previewStatus}>
        <span
          className={preview.status === 'ready' ? styles.statusIconReady : styles.statusIconBlocked}
        >
          {preview.status === 'ready' ? (
            <CheckOne theme='filled' size='16' />
          ) : (
            <CloseOne theme='filled' size='16' />
          )}
        </span>
        <div>
          <strong>
            {t(
              preview.status === 'ready'
                ? 'agentSettings.preview.ready'
                : 'agentSettings.preview.blocked'
            )}
          </strong>
          <span>
            {t('agentSettings.preview.candidateRevision', {
              revision: preview.candidate_revision_ref.revision,
            })}
          </span>
        </div>
      </div>

      <div className={styles.metricGrid}>
        {metrics.map(([key, value]) => (
          <div key={key} className={styles.metric}>
            <span>{t(`agentSettings.preview.metrics.${key}`)}</span>
            <strong>{value}</strong>
          </div>
        ))}
      </div>

      {preview.diagnostics.length > 0 && (
        <div className={styles.diagnosticList}>
          {preview.diagnostics.map((diagnostic, index) => (
            <Alert
              key={index}
              type={diagnostic.severity === 'error' ? 'error' : 'warning'}
              showIcon
              content={previewDiagnosticMessage(diagnostic)}
            />
          ))}
        </div>
      )}

      <Collapse defaultActiveKey={[]} className={styles.inspectorCollapse}>
        <Collapse.Item name='diff' header={t('agentSettings.inspector.revisionDiff')}>
          <div className={styles.diffGrid}>
            <div>
              <span>{t('agentSettings.capabilities.initial')}</span>
              <div className={styles.tagRow}>
                {preview.revision_diff.added_initial.length +
                  preview.revision_diff.removed_initial.length >
                0 ? (
                  <Tag size='small' color='blue'>
                    {preview.revision_diff.added_initial.length +
                      preview.revision_diff.removed_initial.length}{' '}
                    {t('agentSettings.sections.capabilities')}
                  </Tag>
                ) : (
                  <span>{t('agentSettings.common.noChanges')}</span>
                )}
              </div>
            </div>
            <div>
              <span>{t('agentSettings.capabilities.onDemand')}</span>
              <div className={styles.tagRow}>
                {preview.revision_diff.added_on_demand.length +
                  preview.revision_diff.removed_on_demand.length >
                0 ? (
                  <Tag size='small' color='blue'>
                    {preview.revision_diff.added_on_demand.length +
                      preview.revision_diff.removed_on_demand.length}{' '}
                    {t('agentSettings.sections.capabilities')}
                  </Tag>
                ) : (
                  <span>{t('agentSettings.common.noChanges')}</span>
                )}
              </div>
            </div>
          </div>
        </Collapse.Item>

          <Collapse.Item name='snapshot' header={t('agentSettings.inspector.snapshot')}>
            <div className={styles.inspectorRows}>
              <div>
                <span>{t('agentSettings.inspector.snapshot')}</span>
                <strong>
                  {preview.inspector.snapshot_ref
                    ? t('common.added', { defaultValue: 'Resolved' })
                    : t('agentSettings.common.unavailable')}
                </strong>
              </div>
              <div>
                <span>{t('agentSettings.inspector.runtimeProfile')}</span>
                <strong>
                  {preview.inspector.runtime_profile ?? t('agentSettings.common.unavailable')}
                </strong>
              </div>
              <div>
                <span>{t('agentSettings.inspector.protocol')}</span>
                <strong>{preview.inspector.required_runtime_protocol_version}</strong>
              </div>
              <div>
                <span>{t('agentSettings.inspector.tools')}</span>
                <strong>{preview.inspector.tool_schema_refs.length}</strong>
              </div>
              <div>
                <span>{t('agentSettings.inspector.context')}</span>
                <strong>{preview.inspector.context_schema_refs.length}</strong>
              </div>
            </div>
            {preview.inspector.required_runtime_features.length > 0 && (
              <div className={styles.inlineEmpty}>
                {preview.inspector.required_runtime_features.length}{' '}
                {t('agentSettings.inspector.runtimeProfile')}
              </div>
            )}
        </Collapse.Item>

        <Collapse.Item name='continuation' header={t('agentSettings.inspector.continuation')}>
          <div className={styles.policyRow}>
            <Terminal theme='outline' size='16' />
            <div>
              <strong>{t('agentSettings.continuation.title')}</strong>
              <span>{t('agentSettings.continuation.body')}</span>
            </div>
          </div>
        </Collapse.Item>

        <Collapse.Item name='remote' header={t('agentSettings.inspector.remoteToken')}>
          <div className={styles.policyRow}>
            <Connection theme='outline' size='16' />
            <div>
              <strong>
                {t(`agentSettings.remoteToken.status.${tokenState?.status ?? 'unavailable'}`)}
              </strong>
              <span>{t('agentSettings.remoteToken.continuation')}</span>
            </div>
          </div>
        </Collapse.Item>
      </Collapse>
    </div>
  );
};

export default PreviewInspector;
