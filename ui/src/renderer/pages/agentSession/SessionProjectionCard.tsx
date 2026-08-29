import type { SessionCardModel } from './model';
import { Collapse, Tag } from '@arco-design/web-react';
import { Code, MessageOne, Warning, Zap } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { jsonDetails } from './model';
import styles from './AgentSessionPage.module.css';

const icon = (kind: SessionCardModel['kind']) => {
  if (kind === 'message') return <MessageOne theme='outline' size='16' />;
  if (kind === 'tool') return <Code theme='outline' size='16' />;
  if (kind === 'effect') return <Zap theme='outline' size='16' />;
  return <Warning theme='outline' size='16' />;
};

const SessionProjectionCard: React.FC<{ card: SessionCardModel }> = ({ card }) => {
  const { t } = useTranslation();
  if (card.kind === 'message') {
    return (
      <article className={`${styles.messageCard} ${card.role === 'user' ? styles.userMessage : ''}`}>
        <div className={styles.cardHeader}>
          {icon(card.kind)}
          <strong>{t(`agentSettings.session.role.${card.role ?? 'assistant'}`)}</strong>
          <span>#{card.firstSeq}-{card.lastSeq}</span>
        </div>
        <div className={styles.messageContent}>{card.content || t('agentSettings.common.none')}</div>
      </article>
    );
  }
  return (
    <article className={styles.processCard}>
      <div className={styles.cardHeader}>
        {icon(card.kind)}
        <strong>{card.title}</strong>
        {card.state && <Tag size='small' color={card.state === 'uncertain' ? 'orange' : 'gray'}>{card.state}</Tag>}
        <span>#{card.firstSeq}-{card.lastSeq}</span>
      </div>
      {card.details != null && (
        <Collapse className={styles.detailCollapse}>
          <Collapse.Item name='details' header={t('agentSettings.session.details')}>
            <pre>{jsonDetails(card.details)}</pre>
          </Collapse.Item>
        </Collapse>
      )}
    </article>
  );
};

export default SessionProjectionCard;
