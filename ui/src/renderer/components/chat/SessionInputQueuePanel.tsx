import { useEffect, useId, useState, type ReactNode } from 'react';
import { Button, Dropdown, Menu, Modal } from '@arco-design/web-react';
import { DeleteOne, Down, ListView, More, Up } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { MessageId } from '@/common/types/ids';
import type { SessionInputQueueAction, SessionInputQueueItem, SessionInputQueueSnapshot } from '@/common/adapter/ipcBridge';
import styles from './SessionInputQueuePanel.module.css';

type QueueRowProps = {
  item: SessionInputQueueItem;
  index: number;
  count: number;
  changing: boolean;
  available: boolean;
  activeTurn?: MessageId;
  editing: string | undefined;
  queueMenu: ReactNode;
  onEdit: (value: string | undefined) => void;
  onChange: (action: SessionInputQueueAction) => Promise<boolean>;
  move: (index: number, offset: number) => void;
};

function QueueRow({ item, index, count, changing, available, activeTurn, editing, queueMenu, onEdit, onChange, move }: QueueRowProps) {
  const { t } = useTranslation();
  const [inspecting, setInspecting] = useState(false);
  const blocked = item.input_delivery.state === 'blocked';
  const status = [item.held ? t('conversation.inputQueue.held') : null,
    blocked ? t('conversation.inputQueue.blocked') : null,
    item.input_delivery.reason === 'TARGET_TURN_ENDED'
      ? t('conversation.inputDelivery.deferred') : null].filter(Boolean).join(' · ');
  const preview = item.input.content.trim() ? item.input.content : item.input.files?.length
    ? t('conversation.inputQueue.files', { count: item.input.files.length })
    : item.input.inject_skills?.length ? t('conversation.inputQueue.skillCount', { count: item.input.inject_skills.length })
    : t('conversation.inputQueue.emptyInput');
  const save = async () => {
    if (editing === undefined || !editing.trim() || !available) return;
    if (await onChange({ kind: 'replace', message_id: item.message_id, content: editing })) onEdit(undefined);
  };
  const menu = <Menu className={styles.menu} aria-label={t('conversation.inputQueue.more')} onClickMenuItem={key => {
    switch (key) {
      case 'view': setInspecting(true); break;
      case 'edit': onEdit(item.input.content); break;
      case 'up': move(index, -1); break;
      case 'down': move(index, 1); break;
    }
  }}>
    <Menu.Item key='view'>{t('conversation.inputQueue.showFull')}</Menu.Item>
    <Menu.Item key='edit' disabled={changing} aria-disabled={changing}>{t('common.edit')}</Menu.Item>
    <Menu.Item key='up' disabled={changing || index === 0} aria-disabled={changing || index === 0}>{t('conversation.inputQueue.moveUp')}</Menu.Item>
    <Menu.Item key='down' disabled={changing || index === count - 1} aria-disabled={changing || index === count - 1}>{t('conversation.inputQueue.moveDown')}</Menu.Item>
  </Menu>;
  return <li className={`${styles.row} ${editing !== undefined ? styles.editing : ''}`} data-testid='session-input-queue-item'>
    {editing !== undefined ? <div className={styles.editor}>
      <textarea className={styles.editInput} value={editing} disabled={changing}
        aria-label={t('conversation.inputQueue.edit')} onChange={event => onEdit(event.target.value)}
        onKeyDown={event => { if (event.key === 'Escape' && !changing) { event.preventDefault(); onEdit(undefined); } }} />
      <div className={styles.editorFooter}>
        <span className={styles.metadata} role={available ? undefined : 'status'}>
          {t(available ? 'conversation.inputQueue.editPreservesAttachments' : 'conversation.inputQueue.editUnavailable')}
        </span>
        <Button type='text' size='mini' disabled={changing} onClick={() => onEdit(undefined)}>{t('common.cancel')}</Button>
        <Button type='primary' size='mini' disabled={changing || !available || !editing.trim()} onClick={() => void save()}>{t('common.save')}</Button>
      </div>
    </div> : <>
      {count === 1 && <Dropdown trigger='click' position='bl' droplist={queueMenu} getPopupContainer={() => document.body}>
        <Button type='text' size='mini' className={styles.iconAction}
          aria-label={t('conversation.inputQueue.queueActions')} title={t('conversation.inputQueue.queueActions')}
          icon={<ListView theme='outline' size='13' fill='currentColor' />} />
      </Dropdown>}
      <button type='button' className={styles.preview} title={t('conversation.inputQueue.showFull')}
        onClick={() => setInspecting(true)}>{preview}</button>
      {Boolean(item.input.files?.length) && <span className={styles.metadata} title={t('conversation.inputQueue.files', { count: item.input.files!.length })}>
        {t('conversation.inputQueue.files', { count: item.input.files!.length })}
      </span>}
      {status && <span className={styles.state} title={status}>{status}</span>}
      <div className={styles.actions}>
        <Button type='text' size='mini' className={styles.guide} disabled={changing || !activeTurn || blocked}
          title={t('conversation.sendbox.guideHint')}
          onClick={() => activeTurn && void onChange({ kind: 'promote', message_id: item.message_id, turn_id: activeTurn })}>
          <span aria-hidden>↳</span> {t('conversation.inputQueue.guide')}
        </Button>
        <Button type='text' size='mini' className={styles.iconAction} disabled={changing}
          aria-label={t('conversation.inputQueue.remove')} title={t('conversation.inputQueue.remove')}
          icon={<DeleteOne theme='outline' size='13' fill='currentColor' />}
          onClick={() => void onChange({ kind: 'remove', message_id: item.message_id })} />
        <Dropdown trigger='click' position='br' droplist={menu} getPopupContainer={() => document.body}>
          <Button type='text' size='mini' className={styles.iconAction}
            aria-label={t('conversation.inputQueue.more')} title={t('conversation.inputQueue.more')}
            icon={<More theme='outline' size='14' fill='currentColor' />} />
        </Dropdown>
      </div>
    </>}
    {inspecting && <Modal visible title={t('conversation.inputQueue.showFull')} footer={null}
      style={{ width: 'min(640px, calc(100vw - 64px))' }} onCancel={() => setInspecting(false)}>
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

export default function SessionInputQueuePanel({ snapshot, changing, error, activeTurn, onChange, onRefresh }: {
  snapshot: SessionInputQueueSnapshot;
  changing: boolean;
  error?: unknown;
  activeTurn?: MessageId;
  onChange: (action: SessionInputQueueAction) => Promise<unknown>;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();
  const listId = useId();
  const [actionError, setActionError] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState<{ item: SessionInputQueueItem; text: string }>();
  const items = snapshot.items.filter(item => item.input_delivery.placement === 'queued');
  const editingAvailable = editing && items.some(item => item.message_id === editing.item.message_id);
  const visibleItems = editing && !editingAvailable ? [...items, editing.item] : items;
  useEffect(() => {
    if (!items.length) setExpanded(false);
  }, [items.length]);
  const change = async (action: SessionInputQueueAction): Promise<boolean> => {
    setActionError(false);
    try { await onChange(action); return true; }
    catch { setActionError(true); return false; }
  };
  if (!items.length && !editing && !error) return null;
  const held = items.some(item => item.held);
  const listVisible = items.length === 1 || expanded || Boolean(editing);
  const move = (index: number, offset: number) => {
    const ids = snapshot.items.map(item => item.message_id);
    const source = ids.indexOf(items[index].message_id);
    const target = ids.indexOf(items[index + offset].message_id);
    [ids[source], ids[target]] = [ids[target], ids[source]];
    void change({ kind: 'reorder', message_ids: ids });
  };
  const queueMenu = <Menu className={styles.menu} aria-label={t('conversation.inputQueue.queueActions')} onClickMenuItem={key => {
    if (key === 'toggle') void change({ kind: held ? 'resume' : 'pause' });
    if (key === 'clear') void change({ kind: 'clear' });
  }}>
    <Menu.Item key='toggle' disabled={changing} aria-disabled={changing}>{t(held ? 'conversation.inputQueue.resume' : 'conversation.inputQueue.pause')}</Menu.Item>
    <Menu.Item key='clear' disabled={changing} aria-disabled={changing}>{t('conversation.inputQueue.clear')}</Menu.Item>
  </Menu>;
  return <section className={styles.panel} aria-label={editing && !items.length
    ? t('conversation.inputQueue.edit') : t('conversation.inputQueue.title', { count: items.length })} data-testid='session-input-queue'>
    {items.length > 1 && <div className={styles.header}>
      <button type='button' className={styles.toggle} aria-expanded={listVisible} aria-controls={listId}
        disabled={Boolean(editing)} onClick={() => setExpanded(previous => !previous)}>
        <ListView theme='outline' size='13' fill='currentColor' aria-hidden />
        <span>{t('conversation.inputQueue.title', { count: items.length })}</span>
        {listVisible ? <Down theme='outline' size='12' fill='currentColor' aria-hidden /> : <Up theme='outline' size='12' fill='currentColor' aria-hidden />}
      </button>
      <Dropdown trigger='click' position='br' droplist={queueMenu} getPopupContainer={() => document.body}>
        <Button type='text' size='mini' className={styles.iconAction} aria-label={t('conversation.inputQueue.queueActions')}
          title={t('conversation.inputQueue.queueActions')} icon={<More theme='outline' size='14' fill='currentColor' />} />
      </Dropdown>
    </div>}
    {Boolean(error) && <div role='alert' className={styles.error}>
      {t('conversation.inputQueue.loadFailed')} <Button type='text' size='mini' onClick={onRefresh}>{t('common.retry')}</Button>
    </div>}
    {actionError && <p role='alert' className={styles.error}>{t('conversation.inputQueue.changeFailed')}</p>}
    <ol id={listId} hidden={!listVisible} className={styles.list} data-testid='session-input-queue-list'>
      {listVisible && visibleItems.map((item, index) => <QueueRow key={item.message_id} item={item} index={index} count={items.length}
        changing={changing} available={items.some(current => current.message_id === item.message_id)}
        activeTurn={activeTurn} move={move} onChange={change} queueMenu={queueMenu}
        editing={editing?.item.message_id === item.message_id ? editing.text : undefined}
        onEdit={text => setEditing(text === undefined ? undefined : { item, text })} />)}
    </ol>
  </section>;
}
