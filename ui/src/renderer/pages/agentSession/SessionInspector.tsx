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
  return (
    <aside className={styles.inspector}>
      <div className={styles.inspectorTitle}>{t('agentSettings.session.inspector')}</div>
      <div className={styles.inspectorRows}>
        <div><span>{t('agentSettings.session.generation')}</span><code>{head.active_set_generation}</code></div>
        <div><span>{t('agentSettings.inspector.snapshotDigest')}</span><code>{head.snapshot_digest ?? session.agent_binding.resolved_snapshot_ref.snapshot_digest}</code></div>
        <div><span>{t('agentSettings.inspector.protocol')}</span><code>{head.runtime_protocol_version ?? 'n/a'}</code></div>
        <div><span>{t('agentSettings.session.lastSeq')}</span><code>{head.last_seq}</code></div>
        <div><span>{t('agentSettings.session.checkpoint')}</span><code>{head.checkpoint_through_seq ?? 'n/a'}</code></div>
      </div>
      <Collapse className={styles.inspectorCollapse}>
        <Collapse.Item name='active' header={t('agentSettings.session.activeCapabilities')}>
          <div className={styles.tagList}>
            {(capabilities?.active_capabilities ?? []).map((id) => <Tag key={id} size='small' color='green'>{id}</Tag>)}
          </div>
        </Collapse.Item>
        <Collapse.Item name='on-demand' header={t('agentSettings.capabilities.onDemand')}>
          <div className={styles.tagList}>
            {(capabilities?.on_demand_capabilities ?? []).map((id) => <Tag key={id} size='small' color='gray'>{id}</Tag>)}
          </div>
        </Collapse.Item>
        <Collapse.Item name='runtime' header={t('agentSettings.session.runtime')}>
          <div className={styles.inspectorRows}>
            <div><span>runtime_bound_event_id</span><code>{head.runtime_bound_event_id ?? 'n/a'}</code></div>
            <div><span>checkpoint_digest</span><code>{head.runtime_checkpoint_digest ?? 'n/a'}</code></div>
          </div>
        </Collapse.Item>
      </Collapse>
    </aside>
  );
};

export default SessionInspector;
