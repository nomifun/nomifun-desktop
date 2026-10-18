import type { IMessageTips } from '@/common/chat/chatLib';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import MarkdownView from '@/renderer/components/Markdown';
import { emitter } from '@/renderer/utils/emitter';
import { useTranslation } from 'react-i18next';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';

/** Exact canonical question, shared by a parked Turn and a closed handoff. */
export const UserInputRequestNote = ({ message }: { message: IMessageTips }) => {
  const { t } = useTranslation();
  const context = useConversationContextSafe();
  const request = message.content.user_input_request;
  if (!request) return null;
  const canFocus = context?.conversation_id === message.conversation_id && context.readOnly !== true
    && context.hideSendBox !== true && (context.isProcessing !== true || context.executionWait?.turnId === message.turn_id);
  return <div role='note' aria-label={t('conversation.userInputRequest.title')}
    data-testid='user-input-request-note' data-request-id={request.request_id}
    className='bg-1 border-1px border-solid border-[var(--border-base)] rd-8px px-12px py-10px flex flex-col gap-8px'>
    <span className='text-13px font-600 text-t-primary'>{t('conversation.userInputRequest.title')}</span>
    <MarkdownView fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT}>{request.question}</MarkdownView>
    <div className='text-t-secondary'>
      <span className='text-12px'>{t('conversation.userInputRequest.remainingWork')}</span>
      <MarkdownView fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT}>{request.remaining_work}</MarkdownView>
    </div>
    {canFocus && <button type='button' className='w-fit text-12px text-t-secondary hover:text-t-primary cursor-pointer bg-transparent border-none p-0'
      onClick={() => emitter.emit('sendbox.focus', message.conversation_id)}>{t('conversation.userInputRequest.addInput')}</button>}
  </div>;
};
