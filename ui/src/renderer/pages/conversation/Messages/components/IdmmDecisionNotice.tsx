import { useTranslation } from 'react-i18next';
import type { IMessageTips } from '@/common/chat/chatLib';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { emitter } from '@/renderer/utils/emitter';
import { IdmmDecisionExplanation } from './IdmmDecisionExplanation';
import styles from './IdmmDecisionExplanation.module.css';

/** A historical decision outcome, independent of the Session's current runtime state. */
export const IdmmDecisionNotice = ({ message }: { message: IMessageTips }) => {
  const { t } = useTranslation();
  const context = useConversationContextSafe();
  const notice = message.content.idmm_notice;
  if (!notice) return null;
  const waiting = notice.status === 'waiting_for_human';
  return (
    <div className={styles.notice} data-testid='idmm-decision-notice' role='note'>
      <span className={styles.noticeTitle}>{t(waiting ? 'idmm.notice.waitingTitle' : 'idmm.notice.failedTitle')}</span>
      <p className={styles.noticeDescription}>{t(waiting ? 'idmm.notice.waitingDescription' : 'idmm.notice.failedDescription')}</p>
      <IdmmDecisionExplanation decision={notice.decision} conversationId={message.conversation_id} />
      <div className={styles.noticeActions}>
        <time dateTime={new Date(notice.created_at).toISOString()}>{new Date(notice.created_at).toLocaleString()}</time>
        {context?.readOnly !== true && context?.hideSendBox !== true && <button type='button' className={styles.button}
          onClick={() => emitter.emit('sendbox.focus', message.conversation_id)}>{t('idmm.notice.addInput')}</button>}
      </div>
    </div>
  );
};
