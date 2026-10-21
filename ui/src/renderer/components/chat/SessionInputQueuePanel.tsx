import { useState } from 'react';
import { Button } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { MessageId } from '@/common/types/ids';
import type { SessionInputQueueAction, SessionInputQueueItem, SessionInputQueueSnapshot } from '@/common/adapter/ipcBridge';
import styles from './SessionInputQueuePanel.module.css';

function QueueInputText({ content }: { content: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  return <details open={expanded} className={styles.inputText}>
    <summary className={styles.summary} aria-expanded={expanded} onClick={event => {
      event.preventDefault(); setExpanded(value => !value);
    }}>
      <span className={styles.preview}>{content}</span>
      <span className={styles.expandLabel}>{t(expanded ? 'conversation.inputQueue.hideFull' : 'conversation.inputQueue.showFull')}</span>
    </summary>
    {expanded && <pre className={styles.fullText} data-testid='session-input-queue-full-text'>{content}</pre>}
  </details>;
}

export default function SessionInputQueuePanel({ snapshot, changing, error, activeTurn, editDisabled = false, onChange, onEdit, onRefresh }: {
  snapshot: SessionInputQueueSnapshot; changing: boolean; error?: unknown; activeTurn?: MessageId;
  editDisabled?: boolean;
  onChange: (action: SessionInputQueueAction) => Promise<unknown>;
  onEdit: (item: SessionInputQueueItem) => Promise<void>; onRefresh: () => void;
}) {
  const { t } = useTranslation();
  const [actionError, setActionError] = useState(false);
  const run = (operation: () => Promise<unknown>) => { setActionError(false); void operation().catch(() => setActionError(true)); };
  if (!snapshot.items.length && !error) return null;
  const held = snapshot.items.some(item => item.held);
  const move = (index: number, offset: number) => {
    const ids = snapshot.items.map(item => item.message_id);
    [ids[index], ids[index + offset]] = [ids[index + offset], ids[index]];
    run(() => onChange({ kind: 'reorder', message_ids: ids }));
  };
  return <section className={`${styles.panel} mb-8px p-8px rd-12px border b-solid b-[var(--color-border-2)] bg-1`} aria-label={t('conversation.inputQueue.title', { count: snapshot.items.length })} data-testid='session-input-queue'>
    <div className='flex items-center justify-between gap-8px'>
      <span className='text-12px text-t-secondary'>{t('conversation.inputQueue.title', { count: snapshot.items.length })}</span>
      <div className='flex gap-4px'>
        <Button size='mini' disabled={changing || !snapshot.items.length} onClick={() => run(() => onChange({ kind: held ? 'resume' : 'pause' }))}>
          {t(held ? 'conversation.inputQueue.resume' : 'conversation.inputQueue.pause')}
        </Button>
        <Button size='mini' disabled={changing || !snapshot.items.length} onClick={() => run(() => onChange({ kind: 'clear' }))}>{t('conversation.inputQueue.clear')}</Button>
      </div>
    </div>
    {Boolean(error) && <div role='alert' className='text-12px mt-4px'>{t('conversation.inputQueue.loadFailed')} <Button size='mini' onClick={onRefresh}>{t('common.retry')}</Button></div>}
    {actionError && <p role='alert' className='text-12px mt-4px'>{t('conversation.inputQueue.changeFailed')}</p>}
    <ol className={styles.list} data-testid='session-input-queue-list'>
      {snapshot.items.map((item, index) => <li key={item.message_id} className='flex flex-wrap items-center gap-8px py-6px' data-testid='session-input-queue-item'>
        <div className={styles.itemContent}>
          <QueueInputText content={item.input.content} />
          <span className='text-11px text-t-secondary'>
            {t(item.held ? 'conversation.inputQueue.held' : item.input_delivery.state === 'blocked' ? 'conversation.inputQueue.blocked'
              : item.delivery.kind === 'prefer_current' && item.delivery.turn_id === activeTurn ? 'conversation.inputQueue.steering' : 'conversation.inputQueue.queued')}
            {item.input.files?.length ? ` · ${t('conversation.inputQueue.files', { count: item.input.files.length })}` : ''}
          </span>
        </div>
        <div className={`${styles.actions} flex flex-wrap justify-end gap-4px`}>
          <Button size='mini' aria-label={t('conversation.inputQueue.moveUp')} disabled={changing || index === 0} onClick={() => move(index, -1)}>↑</Button>
          <Button size='mini' aria-label={t('conversation.inputQueue.moveDown')} disabled={changing || index === snapshot.items.length - 1} onClick={() => move(index, 1)}>↓</Button>
          <Button size='mini' disabled={changing || !activeTurn || item.input_delivery.state === 'blocked'
            || item.delivery.kind === 'prefer_current' && item.delivery.turn_id === activeTurn}
            title={t('conversation.sendbox.guideHint')}
            onClick={() => activeTurn && run(() => onChange({ kind: 'promote', message_id: item.message_id, turn_id: activeTurn }))}>{t('conversation.inputQueue.guide')}</Button>
          <Button size='mini' disabled={changing || editDisabled} title={editDisabled ? t('conversation.inputQueue.finishDraftFirst') : undefined} onClick={() => run(() => onEdit(item))}>{t('common.edit')}</Button>
          <Button size='mini' disabled={changing} onClick={() => run(() => onChange({ kind: 'remove', message_id: item.message_id }))}>{t('conversation.inputQueue.remove')}</Button>
        </div>
      </li>)}
    </ol>
  </section>;
}
