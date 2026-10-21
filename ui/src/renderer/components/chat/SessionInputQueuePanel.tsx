import { useState } from 'react';
import { Button, Dropdown, Menu, Modal } from '@arco-design/web-react';
import { DeleteOne, ListView, More } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { MessageId } from '@/common/types/ids';
import type { SessionInputQueueAction, SessionInputQueueItem, SessionInputQueueSnapshot } from '@/common/adapter/ipcBridge';
import styles from './SessionInputQueuePanel.module.css';

type QueueActions = {
  onChange: (action: SessionInputQueueAction) => Promise<unknown>;
  onEdit: (item: SessionInputQueueItem) => Promise<void>;
};

type QueueRowProps = QueueActions & {
  item: SessionInputQueueItem; index: number; count: number; changing: boolean; editDisabled: boolean;
  held: boolean; activeTurn?: MessageId; move: (index: number, offset: number) => void;
  run: (operation: () => Promise<unknown>) => void;
};

function QueueRow({ item, index, count, changing, editDisabled, held, activeTurn, move, run, onChange, onEdit }: QueueRowProps) {
  const { t } = useTranslation();
  const [inspecting, setInspecting] = useState(false);
  const blocked = item.input_delivery.state === 'blocked';
  const currentSteering = item.delivery.kind === 'prefer_current' && item.delivery.turn_id === activeTurn;
  const status = [item.held ? t('conversation.inputQueue.held') : null, blocked ? t('conversation.inputQueue.blocked') : null].filter(Boolean).join(' · ');
  const preview = item.input.content.trim() ? item.input.content : item.input.files?.length
    ? t('conversation.inputQueue.files', { count: item.input.files.length })
    : item.input.inject_skills?.length ? t('conversation.inputQueue.skillCount', { count: item.input.inject_skills.length })
    : t('conversation.inputQueue.emptyInput');
  const menuAction = (key: string) => {
    switch (key) {
      case 'view': setInspecting(true); break;
      case 'edit': run(() => onEdit(item)); break;
      case 'up': move(index, -1); break;
      case 'down': move(index, 1); break;
      case 'toggle': run(() => onChange({ kind: held ? 'resume' : 'pause' })); break;
      case 'clear': run(() => onChange({ kind: 'clear' })); break;
    }
  };
  const menu = <Menu className={styles.menu} onClickMenuItem={menuAction}>
    <Menu.Item key='view'>{t('conversation.inputQueue.showFull')}</Menu.Item>
    <Menu.Item key='edit' disabled={changing || editDisabled} aria-disabled={changing || editDisabled}>
      <span title={editDisabled ? t('conversation.inputQueue.finishDraftFirst') : undefined}>{t('common.edit')}</span>
    </Menu.Item>
    <Menu.Item key='up' disabled={changing || index === 0} aria-disabled={changing || index === 0}>{t('conversation.inputQueue.moveUp')}</Menu.Item>
    <Menu.Item key='down' disabled={changing || index === count - 1} aria-disabled={changing || index === count - 1}>{t('conversation.inputQueue.moveDown')}</Menu.Item>
    <Menu.Item key='toggle' disabled={changing} aria-disabled={changing}>{t(held ? 'conversation.inputQueue.resume' : 'conversation.inputQueue.pause')}</Menu.Item>
    <Menu.Item key='clear' disabled={changing} aria-disabled={changing}>{t('conversation.inputQueue.clear')}</Menu.Item>
  </Menu>;
  return <li className={styles.row} data-testid='session-input-queue-item'>
    <span className={styles.queueIcon} aria-hidden title={t(currentSteering ? 'conversation.inputQueue.steering' : 'conversation.inputQueue.queued')}>
      <ListView theme='outline' size='13' fill='currentColor' />
    </span>
    <button type='button' className={styles.preview} title={t('conversation.inputQueue.showFull')}
      onClick={() => setInspecting(true)}>{preview}</button>
    {status && <span className={styles.state} title={status}>{status}</span>}
    <div className={styles.actions}>
      <Button type='text' size='mini' className={styles.guide}
        disabled={changing || !activeTurn || blocked || currentSteering}
        title={t('conversation.sendbox.guideHint')}
        onClick={() => activeTurn && run(() => onChange({ kind: 'promote', message_id: item.message_id, turn_id: activeTurn }))}>
        <span aria-hidden>↳</span> {t('conversation.inputQueue.guide')}
      </Button>
      <Button type='text' size='mini' className={styles.iconAction} disabled={changing}
        aria-label={t('conversation.inputQueue.remove')} title={t('conversation.inputQueue.remove')}
        icon={<DeleteOne theme='outline' size='13' fill='currentColor' />}
        onClick={() => run(() => onChange({ kind: 'remove', message_id: item.message_id }))} />
      <Dropdown trigger='click' position='br' droplist={menu} getPopupContainer={() => document.body}>
        <Button type='text' size='mini' className={styles.iconAction}
          aria-label={t('conversation.inputQueue.more')} title={t('conversation.inputQueue.more')}
          icon={<More theme='outline' size='14' fill='currentColor' />} />
      </Dropdown>
    </div>
    {inspecting && <Modal visible title={t('conversation.inputQueue.showFull')} footer={null}
      style={{ width: 'min(640px, calc(100vw - 64px))' }}
      onCancel={() => setInspecting(false)}>
      <div className={styles.inspection}>
      <pre className={styles.fullText} data-testid='session-input-queue-full-text'>{item.input.content}</pre>
      {Boolean(item.input.files?.length) && <div className={styles.metadata}>
        <span>{t('conversation.inputQueue.files', { count: item.input.files!.length })}</span>
        <ul>{item.input.files!.map((file, fileIndex) => <li key={fileIndex}>{file}</li>)}</ul>
      </div>}
      {Boolean(item.input.inject_skills?.length) && <p className={styles.metadata}>
        {t('conversation.inputQueue.skills')}: {item.input.inject_skills!.join(', ')}
      </p>}
      </div>
    </Modal>}
  </li>;
}

export default function SessionInputQueuePanel({ snapshot, changing, error, activeTurn, editDisabled = false, onChange, onEdit, onRefresh }: QueueActions & {
  snapshot: SessionInputQueueSnapshot; changing: boolean; error?: unknown; activeTurn?: MessageId;
  editDisabled?: boolean; onRefresh: () => void;
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
  return <section className={styles.panel} aria-label={t('conversation.inputQueue.title', { count: snapshot.items.length })} data-testid='session-input-queue'>
    {Boolean(error) && <div role='alert' className={styles.error}>
      {t('conversation.inputQueue.loadFailed')} <Button type='text' size='mini' onClick={onRefresh}>{t('common.retry')}</Button>
    </div>}
    {actionError && <p role='alert' className={styles.error}>{t('conversation.inputQueue.changeFailed')}</p>}
    <ol className={styles.list} data-testid='session-input-queue-list'>
      {snapshot.items.map((item, index) => <QueueRow key={item.message_id} item={item} index={index} count={snapshot.items.length}
        changing={changing} editDisabled={editDisabled} held={held} activeTurn={activeTurn}
        run={run} move={move} onChange={onChange} onEdit={onEdit} />)}
    </ol>
  </section>;
}
