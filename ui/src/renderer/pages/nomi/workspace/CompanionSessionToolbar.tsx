/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Dropdown, Input, Menu, Message, Modal } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import type { ICompanionThread } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { productAgentErrorReason } from '@/renderer/components/agent/ProductAgentBindingSelect';
import styles from './CompanionCohabitView.module.css';

type Props = {
  items: ICompanionThread[];
  selectedId: ConversationId | null;
  selectedName?: string;
  selectedLoaded: boolean;
  entryKind?: 'companion' | 'robot';
  activeDesktopId: ConversationId | null;
  creating: boolean;
  routing: boolean;
  managing: boolean;
  selectedArchived?: boolean;
  clearDisabled?: boolean;
  canCreate: boolean;
  failed: boolean;
  onSelect: (id: ConversationId) => void;
  onCreate: () => void;
  onContinue: () => void;
  onRefresh: () => void;
  onRename: (id: ConversationId, name: string) => Promise<void>;
  onArchive: (id: ConversationId, archived: boolean) => Promise<void>;
  onDelete: (id: ConversationId) => Promise<void>;
  onClear: (id: ConversationId, key: string) => Promise<void>;
};

type SessionDialog = { kind: 'rename' | 'delete' | 'clear'; id: ConversationId; name: string; key: string };

export default function CompanionSessionToolbar(props: Props) {
  const { t } = useTranslation();
  const [showArchived, setShowArchived] = useState(false);
  const [dialog, setDialog] = useState<SessionDialog | null>(null);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const selectedRef = useRef(props.selectedId);
  selectedRef.current = props.selectedId;
  useEffect(() => { setDialog(null); }, [props.selectedId]);
  const selected = props.items.find(item => item.conversation_id === props.selectedId);
  const renameId = selected?.conversation_id ?? (props.selectedLoaded ? props.selectedId : null);
  const selectedName = (props.selectedLoaded ? props.selectedName : undefined) ?? selected?.title ?? '';
  const nameTooLong = dialog?.kind === 'rename' && new TextEncoder().encode(dialog.name.trim()).length > 200;
  const archived = selected?.archived ?? props.selectedArchived ?? false;
  const visibleItems = props.items.filter(item => showArchived || !item.archived || item.conversation_id === props.selectedId);
  const openDialog = (kind: SessionDialog['kind']) => {
    if (props.managing || pendingRef.current) return;
    if (kind === 'rename') {
      if (renameId) setDialog({ kind, id: renameId, name: selectedName, key: uuidv7() });
    } else if (selected) {
      setDialog({ kind, id: selected.conversation_id, name: selected.title, key: uuidv7() });
    }
  };
  const reportError = (error: unknown, fallback: 'conversation.history.renameFailed' | 'nomi.cohabit.sessions.manageFailed') => {
    const key = isBackendHttpError(error) && error.code === 'AGENT_EXECUTION_DELIVERY_PENDING'
      ? 'nomi.cohabit.sessions.deliveryPending'
      : productAgentErrorReason(error) === 'busy' ? 'agentSettings.productBinding.busy' : fallback;
    Message.error(t(key));
  };
  const submitDialog = async () => {
    if (!dialog || pendingRef.current || props.managing || (dialog.kind === 'rename' && (!dialog.name.trim() || nameTooLong))) return;
    const submitted = dialog;
    pendingRef.current = true;
    setPending(true);
    try {
      if (submitted.kind === 'rename') await props.onRename(submitted.id, submitted.name.trim());
      else if (submitted.kind === 'delete') await props.onDelete(submitted.id);
      else await props.onClear(submitted.id, submitted.key);
      if (selectedRef.current === submitted.id) {
        setDialog(null);
        Message.success(t(submitted.kind === 'rename' ? 'conversation.history.renameSuccess'
          : submitted.kind === 'delete' ? 'conversation.history.deleteSuccess' : 'conversation.clearContext.success'));
      }
    } catch (error) {
      if (selectedRef.current === submitted.id) reportError(error,
        submitted.kind === 'rename' ? 'conversation.history.renameFailed' : 'nomi.cohabit.sessions.manageFailed');
    } finally { pendingRef.current = false; setPending(false); }
  };
  const archive = async () => {
    if (!selected || pendingRef.current || props.managing) return;
    const id = selected.conversation_id;
    pendingRef.current = true;
    setPending(true);
    try { await props.onArchive(id, !archived); }
    catch (error) { if (selectedRef.current === id) reportError(error, 'nomi.cohabit.sessions.manageFailed'); }
    finally { pendingRef.current = false; setPending(false); }
  };
  return <div className={styles.sessionToolbar}>
    <NomiSelect size='small' className={styles.sessionSelect} value={props.selectedId ?? undefined}
      aria-label={t('nomi.cohabit.sessions.label')} placeholder={t('nomi.cohabit.sessions.label')}
      onChange={(id: ConversationId) => props.onSelect(id)}>
      {props.selectedId && !selected && <NomiSelect.Option value={props.selectedId}>
        {props.selectedName || t('nomi.cohabit.sessions.current')}
      </NomiSelect.Option>}
      {(['companion', 'robot'] as const).map(role => <NomiSelect.OptGroup key={role} label={t(`nomi.cohabit.sessions.${role}`)}>
        {visibleItems.filter(item => item.role === role).map(item => <NomiSelect.Option key={item.conversation_id} value={item.conversation_id}>
          {(item.conversation_id === props.selectedId ? selectedName : item.title) || t('nomi.cohabit.sessions.untitled')}{item.archived ? ` · ${t('nomi.cohabit.sessions.archived')}` : ''}
        </NomiSelect.Option>)}
      </NomiSelect.OptGroup>)}
    </NomiSelect>
    <Button size='small' type='text' disabled={!renameId || props.managing || pending} onClick={() => openDialog('rename')}>
      {t('conversation.history.rename')}
    </Button>
    <Button size='small' loading={props.creating} disabled={!props.canCreate || props.creating || props.managing} onClick={props.onCreate}>
      {t('nomi.cohabit.sessions.create')}
    </Button>
    {props.selectedId && props.entryKind === 'companion' && <Button size='small' type='text' loading={props.routing}
      disabled={props.routing || props.managing || props.selectedId === props.activeDesktopId} onClick={props.onContinue}>
      {t(props.selectedId === props.activeDesktopId ? 'nomi.cohabit.sessions.windowCurrent' : 'nomi.cohabit.sessions.continueInWindow')}
    </Button>}
    {props.entryKind === 'robot' && <span className={styles.sessionRole}>{t('nomi.cohabit.sessions.robot')}</span>}
    <Dropdown trigger='click' droplist={<Menu>
      <Menu.Item key='archive' disabled={!selected || props.managing || pending} onClick={() => void archive()}>{t(archived ? 'nomi.cohabit.sessions.restore' : 'nomi.cohabit.sessions.archive')}</Menu.Item>
      <Menu.Item key='clear' disabled={!selected || props.clearDisabled || props.managing || pending} onClick={() => openDialog('clear')}>{t('conversation.clearContext.description')}</Menu.Item>
      <Menu.Item key='delete' disabled={!selected || props.managing || pending} onClick={() => openDialog('delete')}>{t('conversation.history.confirmDelete')}</Menu.Item>
      <Menu.Item key='show-archived' onClick={() => setShowArchived(value => !value)}>{t(showArchived ? 'nomi.cohabit.sessions.hideArchived' : 'nomi.cohabit.sessions.showArchived')}</Menu.Item>
    </Menu>}>
      <Button size='small' aria-label={t('common.more')}>{t('common.more')}</Button>
    </Dropdown>
    {props.failed && <span role='alert' className={styles.sessionError}>{t('nomi.cohabit.sessions.failed')}
      <Button size='mini' type='text' onClick={props.onRefresh}>{t('agentSettings.actions.retry')}</Button>
    </span>}
    <Modal visible={dialog !== null} title={t(dialog?.kind === 'rename' ? 'conversation.history.renameTitle'
      : dialog?.kind === 'clear' ? 'conversation.clearContext.description' : 'conversation.history.deleteTitle')}
      confirmLoading={pending} okButtonProps={{ disabled: dialog?.kind === 'rename' && (!dialog.name.trim() || nameTooLong),
        ...(dialog?.kind === 'delete' ? { status: 'danger' as const } : {}) }}
      onOk={() => void submitDialog()} onCancel={() => { if (!pending) setDialog(null); }}>
      {dialog?.kind === 'rename' ? <><Input value={dialog.name} disabled={pending} maxLength={200} autoFocus
        error={nameTooLong} aria-invalid={nameTooLong} aria-describedby={nameTooLong ? 'companion-session-name-error' : undefined}
        aria-label={t('conversation.history.renamePlaceholder')} placeholder={t('conversation.history.renamePlaceholder')}
        onChange={name => setDialog(current => current ? { ...current, name } : current)}
        onPressEnter={event => { if (!event.nativeEvent.isComposing) void submitDialog(); }} />
        {nameTooLong && <p id='companion-session-name-error' role='alert' className={styles.sessionError}>{t('conversation.history.renameTooLong')}</p>}</>
        : <p>{t(dialog?.kind === 'clear' ? 'nomi.cohabit.sessions.clearConfirm' : 'nomi.cohabit.sessions.deleteConfirm')}</p>}
    </Modal>
  </div>;
}
