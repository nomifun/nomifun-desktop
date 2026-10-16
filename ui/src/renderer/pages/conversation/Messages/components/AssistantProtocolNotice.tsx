import { useTranslation } from 'react-i18next';

/** A malformed proposal is a protocol notice, never a file-body disclosure. */
export default function AssistantProtocolNotice() {
  const { t } = useTranslation();
  return <div className='message-protocol-note'>
    {t('messages.toolProtocolError.title', { defaultValue: 'Invalid tool-call format · not executed' })}
  </div>;
}
