import { Button, Tooltip } from '@arco-design/web-react';
import { Help } from '@icon-park/react';
import React, { useId } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './ConnectionHint.module.css';

export default function ConnectionHint({ label, content, id }: { label: string; content: React.ReactNode; id?: string }) {
  const { t } = useTranslation();
  const generatedId = useId();
  const descriptionId = id ?? generatedId;
  return <>
    <Tooltip content={content} trigger={['hover', 'focus']} escToClose>
      <Button className={styles.button} type='text' size='mini' shape='circle' aria-label={t('settings.deviceConnections.helpLabel', { label })} aria-describedby={descriptionId}>
        <Help size={14} className='flex items-center text-t-tertiary' aria-hidden='true' />
      </Button>
    </Tooltip>
    <span id={descriptionId} className='sr-only'>{content}</span>
  </>;
}
