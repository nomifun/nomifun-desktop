import type {
  IAgentSessionCapabilityState,
  IAgentSessionObservation,
} from '@/common/adapter/ipcBridge';
import { Collapse, Tag } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from './AgentSessionPage.module.css';

const SessionInspector: React.FC<{
  observation: IAgentSessionObservation;
  capabilities: IAgentSessionCapabilityState | null;
}> = ({ observation, capabilities }) => {
  const { t } = useTranslation();
  const { head, session } = observation;
  const readOnly =
    observation.continuation?.history_read_only === true ||
    observation.continuation?.can_continue_same_session === false;
  return (
    <aside className={styles.inspector}>
      <div className={styles.inspectorTitle}>{t('agentSettings.session.inspector')}</div>
      <div className={styles.inspectorRows}>
        <div>
          <span>{t('agentSettings.session.runtime')}</span>
          <strong>{head.status}</strong>
        </div>
        <div>
          <span>{t('agentSettings.session.generation')}</span>
          <strong>{head.active_set_generation}</strong>
        </div>
        <div>
          <span>{t('agentSettings.inspector.snapshot')}</span>
          <strong>
            {session.agent_binding.resolved_snapshot_ref
              ? t('common.added', { defaultValue: 'Available' })
              : t('agentSettings.common.unavailable')}
          </strong>
        </div>
        <div>
          <span>{t('agentSettings.session.activeCapabilities')}</span>
          <strong>{capabilities?.active_capabilities.length ?? 0}</strong>
        </div>
        <div>
          <span>{t('agentSettings.session.lastSeq')}</span>
          <strong>{head.last_seq}</strong>
        </div>
      </div>
      {readOnly && (
        <Tag color='orange' size='small'>
          {t('agentSettings.session.readOnly', {
            defaultValue: 'History is read-only',
          })}
        </Tag>
      )}
      <Collapse className={styles.inspectorCollapse}>
        <Collapse.Item name='active' header={t('agentSettings.session.activeCapabilities')}>
          <div className={styles.tagList}>
            {(capabilities?.active_capabilities ?? []).length > 0 ? (
              <Tag size='small' color='green'>
                {capabilities?.active_capabilities.length}{' '}
                {t('agentSettings.session.activeCapabilities')}
              </Tag>
            ) : (
              <span>{t('agentSettings.common.none')}</span>
            )}
          </div>
        </Collapse.Item>
        <Collapse.Item name='runtime' header={t('agentSettings.session.runtime')}>
          <div className={styles.inspectorRows}>
            <div>
              <span>{t('agentSettings.inspector.protocol')}</span>
              <strong>{head.runtime_protocol_version ?? 'n/a'}</strong>
            </div>
            <div>
              <span>{t('agentSettings.session.checkpoint')}</span>
              <strong>
                {head.checkpoint_through_seq == null
                  ? t('agentSettings.common.unavailable')
                  : t('common.added', { defaultValue: 'Available' })}
              </strong>
            </div>
          </div>
        </Collapse.Item>
      </Collapse>
    </aside>
  );
};

export default SessionInspector;
