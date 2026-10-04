import { useTranslation } from 'react-i18next';
import type { ConversationPauseNotice } from '../../utils/conversationRuntime';

export const ExecutionPauseNotice = ({ pause }: { pause: ConversationPauseNotice }) => {
  const { t } = useTranslation();
  const detailKey = !pause.cleanupProven
    ? 'conversation.executionPause.cleanupRequired'
    : pause.reason === 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE'
      ? 'conversation.executionPause.providerUnavailable'
      : 'conversation.executionPause.incomplete';
  return (
    <div role='status' data-testid='execution-pause-notice' className='mb-12px rounded-12px border border-solid border-[var(--border-base)] bg-2 p-12px text-t-primary'>
      <strong>{t('conversation.executionPause.title')}</strong>
      <p className='m-0 mt-4px'>{t(detailKey)}</p>
      <p className='m-0 mt-4px text-12px text-t-secondary'>{t('conversation.executionPause.sendBlocked')}</p>
    </div>
  );
};
