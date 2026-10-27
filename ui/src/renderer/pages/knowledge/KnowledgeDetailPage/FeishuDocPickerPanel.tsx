/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * FeishuDocPickerPanel — right-panel Feishu document browser for feishu-kind
 * knowledge bases.
 *
 * Lists/searches the connected account's documents through the office hub and
 * adds a document to the base's curated name index in one click. Document
 * bodies always stay remote — the base only stores metadata stubs, so the
 * "add" action mirrors the workbench's save-into-base flow.
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Button, Message, Spin } from '@arco-design/web-react';
import {
  Add,
  CheckOne,
  DocDetail,
  FileText,
  Left,
  LinkOne,
  Refresh,
  Search,
} from '@icon-park/react';
import type { KnowledgeBaseId, KnowledgeEntryId } from '@/common/types/ids';
import type { IKnowledgeAddContentResult } from '@/common/adapter/ipcBridge';
import type { OfficeItem } from '@/common/types/officeHub';
import { useOfficeConnection } from '@/renderer/pages/officeHub/useOfficeConnection';
import { useOfficeQuery } from '@/renderer/pages/officeHub/useOfficeHub';
import { AuthorizationPanel, OfficeButton, OfficeDialog } from '@/renderer/pages/officeHub/dialogs';
import {
  hasPermission,
  identityKey,
  officeResourceUrl,
  permissionKey,
  readGroups,
  scopesFor,
} from '@/renderer/pages/officeHub/model';
import '@/renderer/pages/officeHub/styles.css';
import { addKnowledgeContent } from '../useKnowledge';
import { showKnowledgeErrorMessage } from '../KnowledgeErrorFeedback';

/** Kinds a feishu base can index — sheets/bitables stay out of phase one. */
const CURATABLE_KINDS = new Set(['doc', 'docx', 'wiki']);

/** Stable identity shared by search results and curated source entries. */
function feishuDocKey(url: string): string {
  const match = url.match(/\/(docx?|wiki)\/([A-Za-z0-9]+)/i);
  if (match) return `${match[1].toLowerCase()}:${match[2]}`;
  try {
    const parsed = new URL(url);
    parsed.hash = '';
    return parsed.toString().replace(/\/+$/, '').toLowerCase();
  } catch {
    return url.trim().replace(/\/+$/, '').toLowerCase();
  }
}

const itemKey = (item: OfficeItem) => `${item.kind}:${item.id}`;

const resourceTime = (item: OfficeItem): string => {
  const value = item.modified_at;
  if (!value) return '—';
  const time = /^\d+$/.test(value)
    ? new Date(Number(value) * (value.length <= 10 ? 1000 : 1))
    : new Date(value);
  return Number.isNaN(time.getTime()) ? '—' : time.toLocaleDateString();
};

export interface FeishuDocPickerPanelProps {
  knowledgeBaseId: KnowledgeBaseId;
  /** URLs already curated in this base's feishu source. */
  curatedUrls: string[];
  /** Tree folder new stubs are added into; '' is the base root. */
  destinationPath: string;
  /** Stable identity of `destinationPath`, sent alongside when available. */
  destinationEntryId?: KnowledgeEntryId;
  /** True when a tree document is selected — shows a back-to-preview button. */
  hasSelection: boolean;
  onClose?: () => void;
  onAdded: (result: IKnowledgeAddContentResult) => Promise<void>;
}

const FeishuDocPickerPanel: React.FC<FeishuDocPickerPanelProps> = ({
  knowledgeBaseId,
  curatedUrls,
  destinationPath,
  destinationEntryId,
  hasSelection,
  onClose,
  onAdded,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const connection = useOfficeConnection();
  const {
    hub, busy, authVisible, authorize, openExternal, errorPanel,
    authorization, closeAuth, setActionError,
  } = connection;

  const ready =
    hub.status?.runtime.phase === 'ready' &&
    hub.status.connection.enabled &&
    !!hub.status.actor &&
    hub.status.phase === 'ready';
  const canBrowse = ready && hasPermission(hub.status, 'documents.browse');

  const [searchInput, setSearchInput] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  /** Cursor stack for the pages already loaded (page 0 uses a null cursor). */
  const [cursors, setCursors] = useState<{ key: string; stack: Array<string | null> }>({
    key: '',
    stack: [null],
  });
  const [items, setItems] = useState<OfficeItem[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);
  const [adding, setAdding] = useState<Set<string>>(new Set());
  const [addedKeys, setAddedKeys] = useState<Set<string>>(new Set());

  const ownerKey = `${identityKey(hub.status)}:${permissionKey(hub.status)}:${refreshKey}`;
  const pageKey = JSON.stringify([identityKey(hub.status), searchTerm]);
  const cursorStack = cursors.key === pageKey ? cursors.stack : [null];
  const activeCursor = cursorStack[cursorStack.length - 1];

  const documents = useOfficeQuery(
    canBrowse ? { kind: 'documents', query: searchTerm, filter: 'document', cursor: activeCursor } : null,
    ownerKey,
  );

  // Accumulate loaded pages; a fresh query (cursor null) replaces the list.
  useEffect(() => {
    const data = documents.data;
    if (!data) return;
    setItems((previous) => {
      const merged = activeCursor == null ? [] : [...previous];
      const seen = new Set(merged.map(itemKey));
      for (const item of data.items) {
        if (!seen.has(itemKey(item))) {
          seen.add(itemKey(item));
          merged.push(item);
        }
      }
      return merged;
    });
  }, [documents.data, activeCursor]);

  const curatedKeys = useMemo(() => {
    const keys = new Set(addedKeys);
    for (const url of curatedUrls) keys.add(feishuDocKey(url));
    return keys;
  }, [curatedUrls, addedKeys]);

  const rows = items.filter((item) => CURATABLE_KINDS.has(item.kind));
  const nextCursor = documents.data?.next_cursor;

  const resetPages = useCallback(() => {
    setCursors({ key: '', stack: [null] });
    setItems([]);
  }, []);

  const loadMore = () => {
    if (!nextCursor || cursorStack.includes(nextCursor)) return;
    setCursors({ key: pageKey, stack: [...cursorStack, nextCursor] });
  };

  const openResource = async (item: OfficeItem) => {
    try {
      if (!item.url) throw new Error('office.resources.missingLink');
      await openExternal(officeResourceUrl(item.url));
    } catch (error) {
      setActionError(error);
    }
  };

  const addToBase = async (item: OfficeItem) => {
    if (!item.url || adding.has(itemKey(item))) return;
    setAdding((previous) => new Set(previous).add(itemKey(item)));
    try {
      const result = await addKnowledgeContent(knowledgeBaseId, {
        type: 'feishu',
        docs: [{ url: item.url, title: item.title || undefined }],
        destination_parent_path: destinationPath || undefined,
        destination_parent_id: destinationEntryId,
      });
      if (result.type !== 'feishu') throw new Error('Unexpected knowledge content response');
      if (result.added > 0) {
        setAddedKeys((previous) => new Set(previous).add(feishuDocKey(item.url!)));
      }
      // One toast per add: an index failure outranks the success/duplicate
      // states so a half-added doc never looks fully curated.
      if (result.failed > 0) {
        Message.warning(
          t('knowledge.detail.feishu.indexFailed', {
            defaultValue: '文档已加入但索引失败，请稍后刷新',
          }),
        );
      } else if (result.added > 0) {
        Message.success(
          t('knowledge.detail.feishu.addedOk', { defaultValue: '已加入知识库索引' }),
        );
      } else if (result.duplicates > 0) {
        Message.info(
          t('knowledge.detail.feishu.alreadyIn', { defaultValue: '该文档已在知识库中' }),
        );
      }
      await onAdded(result);
    } catch (error) {
      showKnowledgeErrorMessage(error, 'operation', t);
    } finally {
      setAdding((previous) => {
        const next = new Set(previous);
        next.delete(itemKey(item));
        return next;
      });
    }
  };

  const kindLabel = (kind: string) =>
    kind === 'wiki'
      ? t('knowledge.detail.feishu.kindWiki', { defaultValue: '知识库页面' })
      : t('knowledge.detail.feishu.kindDoc', { defaultValue: '文档' });

  const body = () => {
    if (hub.statusLoading && !hub.status) {
      return <p className='knowledge-detail-feishu-empty p-24px text-12px text-[var(--color-text-3)]'>{t('office.loading')}</p>;
    }
    if (!ready) {
      return (
        <div className='knowledge-detail-feishu-empty flex flex-1 flex-col items-center justify-center gap-10px p-24px text-center'>
          <DocDetail theme='outline' size='30' className='text-[var(--color-text-4)]' />
          <div className='text-12px leading-19px text-[var(--color-text-2)]'>
            {t('knowledge.detail.feishu.connectHint', {
              defaultValue: '飞书连接器未就绪，先在飞书工作台完成连接后即可在此挑选文档。',
            })}
          </div>
          <Button size='small' type='primary' onClick={() => navigate('/knowledge/feishu')}>
            {t('knowledge.detail.feishu.connectAction', { defaultValue: '打开飞书工作台' })}
          </Button>
        </div>
      );
    }
    if (!canBrowse) {
      return (
        <div className='knowledge-detail-feishu-empty flex flex-1 flex-col items-center justify-center gap-10px p-24px text-center'>
          <DocDetail theme='outline' size='30' className='text-[var(--color-text-4)]' />
          <div className='text-12px leading-19px text-[var(--color-text-2)]'>
            {t('office.permissions.groups.documents')} · {t('office.permissions.readMissing')}
          </div>
          <Button
            size='small'
            type='primary'
            disabled={busy}
            onClick={() => authorize(scopesFor(hub.status, readGroups.documents))}
          >
            {t('office.actions.authorizeRead')}
          </Button>
        </div>
      );
    }
    if (documents.error) {
      return (
        <div className='p-12px'>
          {errorPanel(documents.error)}
          <OfficeButton onClick={documents.refresh}>{t('office.actions.check')}</OfficeButton>
        </div>
      );
    }
    if (documents.loading && items.length === 0) {
      return (
        <div className='flex flex-1 items-center justify-center p-24px'>
          <Spin size={20} />
        </div>
      );
    }
    return (
      <>
        <div className='min-h-0 flex-1 overflow-y-auto'>
          {rows.length === 0 && (
            <div className='knowledge-detail-feishu-empty flex h-full flex-col items-center justify-center gap-6px p-24px text-center'>
              <FileText theme='outline' size='30' className='text-[var(--color-text-4)]' />
              <div className='text-12px text-[var(--color-text-3)]'>
                {searchTerm
                  ? t('knowledge.detail.feishu.emptySearch', { defaultValue: '没有匹配的飞书文档' })
                  : t('knowledge.detail.feishu.empty', { defaultValue: '暂无可索引的飞书文档' })}
              </div>
            </div>
          )}
          {rows.map((item) => {
            const key = itemKey(item);
            const curated = !!item.url && curatedKeys.has(feishuDocKey(item.url));
            return (
              <div
                key={key}
                className='group flex items-center gap-9px border-0 border-b border-solid border-[var(--color-border-3)] px-14px py-9px transition-colors hover:bg-[var(--color-fill-1)]'
              >
                <span className='grid size-30px shrink-0 place-items-center rounded-8px bg-[var(--color-fill-2)] text-[var(--color-text-2)]'>
                  <FileText theme='outline' size='15' />
                </span>
                <button
                  type='button'
                  className='min-w-0 flex-1 cursor-pointer border-0 bg-transparent p-0 text-left font-[inherit] disabled:cursor-default'
                  disabled={!item.url}
                  title={t(item.url ? 'office.resources.browserHint' : 'office.resources.missingLink')}
                  onClick={() => void openResource(item)}
                >
                  <span className='block truncate text-12px font-600 leading-18px text-[var(--color-text-1)]'>
                    {item.title || item.id}
                  </span>
                  <span className='block text-10px leading-16px text-[var(--color-text-3)]'>
                    {kindLabel(item.kind)} · {resourceTime(item)}
                  </span>
                </button>
                {curated ? (
                  <span className='inline-flex shrink-0 items-center gap-4px rounded-6px bg-[var(--color-success-light-1)] px-7px py-3px text-10px font-600 text-success-6'>
                    <CheckOne theme='outline' size='11' />
                    {t('knowledge.detail.feishu.alreadyIn', { defaultValue: '已在知识库' })}
                  </span>
                ) : (
                  <Button
                    size='mini'
                    type='primary'
                    icon={<Add theme='outline' size='12' />}
                    disabled={!item.url}
                    loading={adding.has(key)}
                    onClick={() => void addToBase(item)}
                  >
                    {t('knowledge.detail.feishu.add', { defaultValue: '加入知识库' })}
                  </Button>
                )}
              </div>
            );
          })}
          {documents.loading && items.length > 0 && (
            <div className='flex items-center justify-center p-10px'>
              <Spin size={16} />
            </div>
          )}
        </div>
        {(documents.data?.has_more || nextCursor) && (
          <div className='flex shrink-0 items-center justify-center border-0 border-t border-solid border-[var(--color-border-3)] px-10px py-8px'>
            <Button
              size='small'
              disabled={!nextCursor || documents.loading}
              loading={documents.loading}
              onClick={loadMore}
            >
              {t('knowledge.detail.feishu.loadMore', { defaultValue: '加载更多' })}
            </Button>
          </div>
        )}
      </>
    );
  };

  return (
    <div className='knowledge-detail-feishu-picker flex h-full min-h-0 flex-col'>
      {/* Panel header: title + search + actions */}
      <div className='knowledge-doc-divider-bottom flex shrink-0 items-center gap-8px px-14px py-10px'>
        {hasSelection && onClose && (
          <Button
            type='text'
            size='mini'
            shape='circle'
            icon={<Left theme='outline' size='13' />}
            aria-label={t('knowledge.detail.feishu.backToPreview', { defaultValue: '返回预览' })}
            onClick={onClose}
          />
        )}
        <div className='min-w-0'>
          <div className='text-12px font-700 leading-18px text-[var(--color-text-1)]'>
            {t('knowledge.detail.feishu.pickerTitle', { defaultValue: '飞书文档' })}
          </div>
        </div>
        <form
          className='ml-auto flex min-w-0 flex-1 items-center gap-6px rounded-7px bg-[var(--color-fill-2)] border border-solid border-[var(--color-border-3)] px-9px py-4px'
          onSubmit={(event) => {
            event.preventDefault();
            setSearchTerm(searchInput.trim());
            resetPages();
          }}
        >
          <Search theme='outline' size='12' className='shrink-0 text-[var(--color-text-3)]' />
          <input
            className='min-w-0 w-full border-none bg-transparent text-11px text-[var(--color-text-1)] outline-none placeholder:text-[var(--color-text-3)]'
            placeholder={t('knowledge.detail.feishu.searchPlaceholder', {
              defaultValue: '搜索飞书文档…',
            })}
            aria-label={t('office.resources.search')}
            value={searchInput}
            onChange={(event) => setSearchInput(event.target.value)}
          />
        </form>
        <Button
          type='text'
          size='mini'
          shape='circle'
          icon={<Refresh theme='outline' size='13' />}
          aria-label={t('office.actions.refresh')}
          loading={documents.loading || hub.statusLoading || busy}
          onClick={() => {
            setRefreshKey((value) => value + 1);
            resetPages();
          }}
        />
        <Button
          type='text'
          size='mini'
          shape='circle'
          icon={<LinkOne theme='outline' size='13' />}
          aria-label={t('knowledge.detail.feishu.openWorkbench', { defaultValue: '飞书工作台' })}
          onClick={() => navigate('/knowledge/feishu')}
        />
      </div>
      <div className='px-14px pt-8px text-10px leading-16px text-[var(--color-text-3)]'>
        {t('knowledge.detail.feishu.pickerHint', {
          defaultValue: '一键把文档加入知识库名称索引；正文不落本地，检索与阅读经飞书连接器实时进行。',
        })}
      </div>
      <div className='px-14px pt-2px text-10px leading-16px text-[var(--color-text-3)]'>
        {t('knowledge.detail.feishu.destination', {
          path:
            destinationPath ||
            t('knowledge.detail.feishu.destinationRoot', { defaultValue: '根目录' }),
          defaultValue: '加入到：{{path}}',
        })}
      </div>
      {errorPanel(connection.actionError)}
      <div className='flex min-h-0 flex-1 flex-col pt-6px'>{body()}</div>
      {authVisible && (
        <OfficeDialog
          title={t('office.auth.title')}
          onClose={closeAuth}
          busy={busy}
          footer={
            <OfficeButton primary disabled={busy} onClick={closeAuth}>
              {t('office.auth.return')}
            </OfficeButton>
          }
        >
          <AuthorizationPanel {...authorization} />
        </OfficeDialog>
      )}
    </div>
  );
};

export default FeishuDocPickerPanel;
