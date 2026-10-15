import { Button, Input } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { useConversationExport } from '@/renderer/hooks/file/useConversationExport';
import { useCompositionInput } from '@/renderer/hooks/chat/useCompositionInput';
import SlashCommandMenu from '../SlashCommandMenu';

export function ConversationExportPanel({ flow }: { flow: ReturnType<typeof useConversationExport> }) {
  const { t } = useTranslation();
  const { compositionHandlers, isImeActive } = useCompositionInput();
  if (flow.step === 'menu') return <SlashCommandMenu title={t('messages.export.menuTitle')} hint={t('messages.export.menuHint')}
    items={flow.menuItems} activeIndex={flow.activeIndex} loading={flow.loading} onHoverItem={flow.setActiveIndex}
    onSelectItem={item => flow.onSelectMenuItem(item.key)} emptyText={t('messages.slash.empty')} />;
  if (flow.step !== 'filename') return null;
  return <div className='rounded-14px border border-solid overflow-hidden p-12px flex flex-col gap-10px'
    style={{ borderColor: 'var(--color-border-2)', background: 'var(--color-bg-1)' }}>
    <div className='text-13px font-semibold text-t-primary'>{t('messages.export.fileNameLabel')}</div>
    <Input autoFocus value={flow.filename} onChange={flow.setFilename}
      placeholder={t('messages.export.fileNamePlaceholder')} disabled={flow.loading} {...compositionHandlers}
      onKeyDown={event => { if (!isImeActive(event)) flow.handleKeyDown(event); }} />
    <div className='text-12px text-t-secondary break-all'>{t('messages.export.pathLabel')}: {flow.pathPreview}</div>
    <div className='flex items-center justify-end gap-8px'>
      <Button size='small' type='secondary' disabled={flow.loading} onClick={flow.closeExportFlow}>{t('common.cancel')}</Button>
      <Button size='small' type='secondary' disabled={flow.loading} onClick={flow.showMenu}>{t('common.back')}</Button>
      <Button size='small' type='primary' loading={flow.loading} onClick={() => void flow.submitFilename()}>{t('common.save')}</Button>
    </div>
  </div>;
}
