import { useState } from 'react';
import { useTranslation } from 'react-i18next';

/** Historical malformed output remains inspectable without entering prose. */
export default function AssistantProtocolNotice({ raw }: { raw: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  return (
    <details className='message-protocol-note' onToggle={(event) => setExpanded(event.currentTarget.open)}>
      <summary>{t('messages.toolProtocolError.title', { defaultValue: 'Invalid tool-call format · not executed' })}</summary>
      {expanded && <div className='message-protocol-note__detail'>
        <p>{t('messages.toolProtocolError.detail', {
          defaultValue: 'The model emitted a tool request as text. It was not executed. The original record is available below.',
        })}</p>
        <pre>{raw}</pre>
      </div>}
    </details>
  );
}
