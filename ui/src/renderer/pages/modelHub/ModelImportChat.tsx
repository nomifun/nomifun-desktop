import React, { useState } from 'react';
import { Button } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useNomiQuickStart } from '@/renderer/hooks/agent/useNomiQuickStart';

/** Start with an editable draft and the current official General Agent grant. */
const ModelImportChat: React.FC = () => {
  const { t } = useTranslation();
  const { start, canStart } = useNomiQuickStart();
  const [starting, setStarting] = useState(false);
  const open = async () => {
    if (starting) return;
    setStarting(true);
    try {
      await start({ name: t('settings.modelHub.importChatTitle'), prompt: t('settings.modelHub.importChatPrompt'), send: false });
    } finally {
      setStarting(false);
    }
  };
  return (
    <div className='mb-16px flex flex-wrap items-center gap-8px'>
      <Button type='outline' loading={starting} disabled={!canStart} onClick={() => void open()}>
        {t('settings.modelHub.importChatAction')}
      </Button>
      <span className='text-12px leading-18px text-t-tertiary'>
        {t(canStart ? 'settings.modelHub.importChatHint' : 'settings.modelHub.importChatNeedsModel')}
      </span>
    </div>
  );
};

export default ModelImportChat;
