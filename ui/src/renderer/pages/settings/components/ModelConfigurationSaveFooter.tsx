import { Button, Tooltip } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';

export interface ModelConfigurationSaveFooterProps {
  configuring: boolean;
  saving: boolean;
  disabled: boolean;
  onCancel: () => void;
  onSave: () => void;
  onForceSave: () => void;
  confirmText?: string;
}

export default function ModelConfigurationSaveFooter({ configuring, saving, disabled, onCancel, onSave, onForceSave, confirmText }: ModelConfigurationSaveFooterProps) {
  const { t } = useTranslation();
  return <div className='flex items-center justify-end gap-8px'>
    <Button htmlType='button' disabled={saving} className='min-w-72px px-16px' style={{ borderRadius: 8 }} onClick={() => {
      if (!saving) onCancel();
    }}>{t('common.cancel')}</Button>
    {configuring && <Tooltip content={t('settings.modelProtocolDiscovery.forceSaveHint')}>
      <Button htmlType='button' type='text' disabled={disabled || saving} className='min-w-72px px-16px' style={{ borderRadius: 8 }} onClick={() => {
        if (configuring && !disabled && !saving) onForceSave();
      }}>{t('settings.modelProtocolDiscovery.forceSave')}</Button>
    </Tooltip>}
    <Button htmlType='button' type='primary' loading={saving || configuring} disabled={disabled || configuring || saving}
      aria-busy={saving || configuring} className='min-w-72px px-16px' style={{ borderRadius: 8 }} onClick={() => {
        if (!disabled && !configuring && !saving) onSave();
      }}>{confirmText ?? t('common.confirm')}</Button>
  </div>;
}
