/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Checkbox, Drawer, Dropdown, Empty, Input, Menu, Message, Modal, Pagination, Radio, Select, Spin, Tag, Tooltip } from '@arco-design/web-react';
import { More, Pin } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type {
  ICompanionMemory,
  ICompanionMemoryBatchAction,
  ICompanionMemoryKind,
  ICompanionMemoryMergeGroup,
  ICompanionMemorySort,
} from '@/common/adapter/ipcBridge';
import type { CompanionId, CompanionMemoryId } from '@/common/types/ids';
import { parseSnippetSegments } from './memorySnippet';

const KINDS = ['profile', 'preference', 'knowledge', 'episode', 'task', 'affective'] as const;

const KIND_COLORS: Record<string, string> = {
  profile: 'gray',
  preference: 'pinkpurple',
  knowledge: 'green',
  episode: 'orange',
  task: 'red',
  affective: 'purple',
};

type ScopeKind = 'user' | 'companion';

interface CompanionRef {
  companion_id: CompanionId;
  name: string;
}

/** Per-group editable state of the merge assistant drawer. */
interface MergeDraft {
  ids: CompanionMemoryId[];
  content: string;
  kind: ICompanionMemoryKind;
}

interface MemoriesTabProps {
  /** The companion currently selected on the nomi page; scopes the default view. */
  companionId?: CompanionId | null;
  /** Roster, for the scope selector + per-row owner badges. */
  companions?: CompanionRef[];
}

const MemoriesTab: React.FC<MemoriesTabProps> = ({ companionId = null, companions = [] }) => {
  const { t } = useTranslation();
  const [memories, setMemories] = useState<ICompanionMemory[]>([]);
  const [loading, setLoading] = useState(true);
  const [kind, setKind] = useState<string>('');
  const [q, setQ] = useState('');
  const [memStatus, setMemStatus] = useState<'active' | 'archived'>('active');
  const [sort, setSort] = useState<ICompanionMemorySort>('relevance');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(10);
  const [total, setTotal] = useState(0);
  // 'self' = shared + this companion's private (default when a companion is
  // selected); 'all' = every companion's memories (cross-companion view).
  const [scopeMode, setScopeMode] = useState<'self' | 'all'>(companionId ? 'self' : 'all');

  const [selected, setSelected] = useState<CompanionMemoryId[]>([]);
  const [reclassifyVisible, setReclassifyVisible] = useState(false);
  const [reclassifyKind, setReclassifyKind] = useState<ICompanionMemoryKind>('knowledge');

  const [mergeVisible, setMergeVisible] = useState(false);
  const [mergeLoading, setMergeLoading] = useState(false);
  const [mergeGroups, setMergeGroups] = useState<ICompanionMemoryMergeGroup[]>([]);
  const [mergeDrafts, setMergeDrafts] = useState<MergeDraft[]>([]);

  const [addVisible, setAddVisible] = useState(false);
  const [addKind, setAddKind] = useState<string>('knowledge');
  const [addContent, setAddContent] = useState('');
  const [addScopeKind, setAddScopeKind] = useState<ScopeKind>(companionId ? 'companion' : 'user');
  const [addScopeCompanionId, setAddScopeCompanionId] = useState<CompanionId | null>(companionId);

  const [editTarget, setEditTarget] = useState<ICompanionMemory | null>(null);
  const [editContent, setEditContent] = useState('');
  const [editScopeKind, setEditScopeKind] = useState<ScopeKind>('user');
  const [editScopeCompanionId, setEditScopeCompanionId] = useState<CompanionId | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ICompanionMemory | null>(null);

  const companionName = useCallback(
    (id: CompanionId) => companions.find((c) => c.companion_id === id)?.name || id,
    [companions]
  );

  const refreshSeq = useRef(0);

  const refresh = useCallback(async () => {
    const seq = ++refreshSeq.current;
    setLoading(true);
    try {
      const result = await ipcBridge.companion.listMemories.invoke({
        kind: kind || undefined,
        q: q || undefined,
        status: memStatus,
        // 'self' scopes to shared + selected companion's private; 'all' omits
        // the filter so every companion's memories show.
        scope_companion_id: scopeMode === 'self' && companionId ? companionId : undefined,
        sort,
        limit: pageSize,
        offset: (page - 1) * pageSize,
      });
      // Out-of-order guard: a slow stale response must not clobber the
      // results of a newer query (rapid typing fires overlapping requests).
      if (seq === refreshSeq.current) {
        const maxPage = Math.max(1, Math.ceil(result.total / pageSize));
        setTotal(result.total);
        // A deletion can leave the current page past the end. Keep the existing
        // rows visible while the next request loads the last valid page.
        if (page > maxPage) {
          setPage(maxPage);
          return;
        }
        setMemories(result.items);
      }
    } catch (e) {
      if (seq === refreshSeq.current) Message.error(String(e));
    } finally {
      if (seq === refreshSeq.current) setLoading(false);
    }
  }, [kind, q, memStatus, sort, scopeMode, companionId, page, pageSize]);

  // Debounce refetches slightly so typing does not create overlapping requests.
  useEffect(() => {
    const timer = setTimeout(() => void refresh(), 250);
    return () => clearTimeout(timer);
  }, [refresh]);

  // A new result set always begins at its first page. Page navigation itself
  // changes only `page`, so it keeps the current filters intact.
  useEffect(() => {
    setPage(1);
  }, [kind, q, memStatus, sort, scopeMode, companionId, pageSize]);

  // Selection is per result set: filter or page changes drop it.
  useEffect(() => {
    setSelected([]);
  }, [kind, q, memStatus, sort, scopeMode, companionId, page, pageSize]);

  // nomi can save/edit/delete memories mid-chat or from another surface —
  // reflect them live.
  useEffect(() => {
    const unsubs = [
      ipcBridge.companion.onMemoryCreated.on(() => void refresh()),
      ipcBridge.companion.onMemoryUpdated.on(() => void refresh()),
      ipcBridge.companion.onMemoryDeleted.on(() => void refresh()),
    ];
    return () => unsubs.forEach((u) => u());
  }, [refresh]);

  const togglePin = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.updateMemory.invoke({ memory_id: m.memory_id, pinned: !m.pinned });
      void refresh();
    },
    [refresh]
  );

  const toggleArchive = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.updateMemory.invoke({
        memory_id: m.memory_id,
        status: m.status === 'active' ? 'archived' : 'active',
      });
      void refresh();
    },
    [refresh]
  );

  const remove = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.deleteMemory.invoke({ memory_id: m.memory_id });
      void refresh();
    },
    [refresh]
  );

  const confirmRemove = useCallback(async () => {
    if (!deleteTarget) return;
    await remove(deleteTarget);
    setDeleteTarget(null);
  }, [deleteTarget, remove]);

  // ── batch operations ──

  const toggleSelected = useCallback((id: CompanionMemoryId) => {
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }, []);

  const pageIds = memories.map((m) => m.memory_id);
  const allSelected = pageIds.length > 0 && pageIds.every((id) => selected.includes(id));

  const toggleSelectAll = useCallback(() => {
    setSelected(allSelected ? [] : memories.map((m) => m.memory_id));
  }, [allSelected, memories]);

  const runBatch = useCallback(
    async (action: ICompanionMemoryBatchAction, batchKind?: ICompanionMemoryKind) => {
      try {
        await ipcBridge.companion.batchMemories.invoke({ ids: selected, action, kind: batchKind });
        Message.success(t('nomi.memories.batchDone'));
        setSelected([]);
        void refresh();
      } catch (e) {
        Message.error(String(e));
      }
    },
    [selected, refresh, t]
  );

  const confirmBatch = useCallback(
    (action: 'archive' | 'restore' | 'delete') => {
      const title =
        action === 'archive'
          ? t('nomi.memories.batchArchiveConfirm', { count: selected.length })
          : action === 'restore'
            ? t('nomi.memories.batchRestoreConfirm', { count: selected.length })
            : t('nomi.memories.batchDeleteConfirm', { count: selected.length });
      Modal.confirm({
        title,
        okButtonProps: action === 'delete' ? { status: 'danger' } : undefined,
        onOk: () => void runBatch(action),
      });
    },
    [runBatch, selected.length, t]
  );

  const submitReclassify = useCallback(async () => {
    await runBatch('reclassify', reclassifyKind);
    setReclassifyVisible(false);
  }, [runBatch, reclassifyKind]);

  // ── merge assistant ──

  const openMerge = useCallback(async () => {
    setMergeVisible(true);
    setMergeLoading(true);
    try {
      const groups = await ipcBridge.companion.memoryMergeSuggestions.invoke();
      setMergeGroups(groups);
      setMergeDrafts(
        groups.map((group) => ({
          ids: group.memories.map((m) => m.memory_id),
          // Pre-fill with the longest member; the user edits before confirming.
          content: group.memories.reduce((best, m) => (m.content.length > best.length ? m.content : best), ''),
          kind: group.memories[0]?.kind ?? 'knowledge',
        }))
      );
    } catch (e) {
      Message.error(String(e));
    } finally {
      setMergeLoading(false);
    }
  }, []);

  const patchDraft = useCallback((index: number, patch: Partial<MergeDraft>) => {
    setMergeDrafts((drafts) => drafts.map((draft, i) => (i === index ? { ...draft, ...patch } : draft)));
  }, []);

  const submitMerge = useCallback(
    async (index: number) => {
      const draft = mergeDrafts[index];
      if (!draft || draft.ids.length < 2 || !draft.content.trim()) return;
      try {
        await ipcBridge.companion.mergeMemories.invoke({
          group: draft.ids,
          merged_content: draft.content.trim(),
          kind: draft.kind,
        });
        Message.success(t('nomi.memories.merged'));
        setMergeGroups((groups) => groups.filter((_, i) => i !== index));
        setMergeDrafts((drafts) => drafts.filter((_, i) => i !== index));
        void refresh();
      } catch (e) {
        Message.error(String(e));
      }
    },
    [mergeDrafts, refresh, t]
  );

  // ── add / edit ──

  const openAdd = useCallback(() => {
    setAddKind('knowledge');
    setAddContent('');
    setAddScopeKind(companionId ? 'companion' : 'user');
    setAddScopeCompanionId(companionId);
    setAddVisible(true);
  }, [companionId]);

  const add = useCallback(async () => {
    if (!addContent.trim()) return;
    try {
      await ipcBridge.companion.addMemory.invoke({
        kind: addKind,
        content: addContent.trim(),
        // Omitted = shared; a canonical companion id = private to it.
        scope_companion_id: addScopeKind === 'companion' ? (addScopeCompanionId ?? undefined) : undefined,
      });
      setAddVisible(false);
      setAddContent('');
      void refresh();
      Message.success(t('nomi.memories.added'));
    } catch (e) {
      Message.error(String(e));
    }
  }, [addKind, addContent, addScopeKind, addScopeCompanionId, refresh, t]);

  const openEdit = useCallback((m: ICompanionMemory) => {
    setEditTarget(m);
    setEditContent(m.content);
    setEditScopeKind(m.scope_kind === 'companion' ? 'companion' : 'user');
    setEditScopeCompanionId(m.scope_companion_id);
  }, []);

  const saveEdit = useCallback(async () => {
    if (!editTarget || !editContent.trim()) return;
    try {
      await ipcBridge.companion.updateMemory.invoke({
        memory_id: editTarget.memory_id,
        content: editContent.trim(),
        scope_kind: editScopeKind,
        scope_companion_id: editScopeKind === 'companion' ? (editScopeCompanionId ?? undefined) : undefined,
      });
      setEditTarget(null);
      void refresh();
      Message.success(t('nomi.memories.saved'));
    } catch (e) {
      Message.error(String(e));
    }
  }, [editTarget, editContent, editScopeKind, editScopeCompanionId, refresh, t]);

  // A private scope requires a chosen companion; disable the OK button otherwise.
  const addInvalid = !addContent.trim() || (addScopeKind === 'companion' && !addScopeCompanionId);
  const editInvalid = !editContent.trim() || (editScopeKind === 'companion' && !editScopeCompanionId);

  const scopeSelector = (
    scopeKind: ScopeKind,
    scopeCompanionId: CompanionId | null,
    setScopeKind: (k: ScopeKind) => void,
    setScopeCompanionId: (id: CompanionId | null) => void
  ) => (
    <div className='flex items-center gap-8px flex-wrap'>
      <Radio.Group
        type='button'
        size='small'
        value={scopeKind}
        onChange={(v: ScopeKind) => {
          setScopeKind(v);
          if (v === 'companion' && !scopeCompanionId && companionId) setScopeCompanionId(companionId);
        }}
      >
        <Radio value='user'>{t('nomi.memories.scopeShared')}</Radio>
        <Radio value='companion'>{t('nomi.memories.scopePrivate')}</Radio>
      </Radio.Group>
      {scopeKind === 'companion' && (
        <Select
          size='small'
          style={{ width: 180 }}
          value={scopeCompanionId || undefined}
          onChange={setScopeCompanionId}
          placeholder={t('nomi.memories.scopePickCompanion')}
        >
          {companions.map((c) => (
            <Select.Option key={c.companion_id} value={c.companion_id}>
              {c.name || c.companion_id}
            </Select.Option>
          ))}
        </Select>
      )}
    </div>
  );

  const scopeBadge = (m: ICompanionMemory) =>
    m.scope_kind === 'companion' ? (
      <Tag color='arcoblue' bordered>
        {t('nomi.memories.scopePrivateOf', { name: companionName(m.scope_companion_id!) })}
      </Tag>
    ) : (
      <Tag bordered>{t('nomi.memories.scopeShared')}</Tag>
    );

  /** Memory content with FTS hit highlighting (whitelist `<b>` parsing only). */
  const memoryContent = (m: ICompanionMemory) =>
    m.snippet ? (
      <>
        {parseSnippetSegments(m.snippet).map((segment, index) =>
          segment.hit ? (
            <b key={index} className='text-[rgb(var(--primary-6))] font-semibold'>
              {segment.text}
            </b>
          ) : (
            <React.Fragment key={index}>{segment.text}</React.Fragment>
          )
        )}
      </>
    ) : (
      m.content
    );

  const memoryActionMenu = (m: ICompanionMemory) => (
    <Menu
      onClickMenuItem={(key) => {
        if (key === 'edit') {
          openEdit(m);
          return;
        }
        if (key === 'archive') {
          void toggleArchive(m);
          return;
        }
        if (key === 'delete') setDeleteTarget(m);
      }}
    >
      <Menu.Item key='edit'>{t('nomi.memories.edit')}</Menu.Item>
      <Menu.Item key='archive'>{m.status === 'active' ? t('nomi.memories.archive') : t('nomi.memories.restore')}</Menu.Item>
      <Menu.Item key='delete' className='!text-[rgb(var(--danger-6))]'>
        {t('nomi.memories.delete')}
      </Menu.Item>
    </Menu>
  );

  const handlePageChange = useCallback(
    (nextPage: number, nextPageSize: number) => {
      const pageSizeChanged = nextPageSize !== pageSize;
      if (pageSizeChanged) setPageSize(nextPageSize);
      setPage(pageSizeChanged ? 1 : nextPage);
    },
    [pageSize]
  );

  const initialLoading = loading && memories.length === 0 && total === 0;

  return (
    <div className='flex flex-col gap-12px py-8px'>
      <div className='flex gap-8px flex-wrap items-center'>
        <Radio.Group type='button' value={memStatus} onChange={(v: 'active' | 'archived') => setMemStatus(v)}>
          <Radio value='active'>{t('nomi.memories.statusActive')}</Radio>
          <Radio value='archived'>{t('nomi.memories.statusArchived')}</Radio>
        </Radio.Group>
        <Select style={{ width: 140 }} value={kind} onChange={setKind} placeholder={t('nomi.memories.kindAll')}>
          <Select.Option value=''>{t('nomi.memories.kindAll')}</Select.Option>
          {KINDS.map((k) => (
            <Select.Option key={k} value={k}>
              {t(`nomi.kinds.${k}`)}
            </Select.Option>
          ))}
        </Select>
        {companionId && (
          <Radio.Group type='button' size='small' value={scopeMode} onChange={(v: 'self' | 'all') => setScopeMode(v)}>
            <Radio value='self'>{t('nomi.memories.scopeFilterSelf')}</Radio>
            <Radio value='all'>{t('nomi.memories.scopeFilterAll')}</Radio>
          </Radio.Group>
        )}
        <Input.Search
          style={{ width: 220 }}
          placeholder={t('nomi.memories.searchPlaceholder')}
          value={q}
          onChange={setQ}
          allowClear
        />
        <Select style={{ width: 120 }} value={sort} onChange={(v: ICompanionMemorySort) => setSort(v)}>
          <Select.Option value='relevance'>{t('nomi.memories.sortRelevance')}</Select.Option>
          <Select.Option value='time'>{t('nomi.memories.sortTime')}</Select.Option>
          <Select.Option value='importance'>{t('nomi.memories.sortImportance')}</Select.Option>
        </Select>
        <Button onClick={() => void openMerge()}>{t('nomi.memories.merge')}</Button>
        <Button type='primary' onClick={openAdd}>
          {t('nomi.memories.add')}
        </Button>
      </div>

      {memories.length > 0 && (
        <div className='flex items-center gap-10px flex-wrap rounded-8px bg-fill-1 px-10px py-6px'>
          <Checkbox checked={allSelected} onChange={toggleSelectAll}>
            {t('nomi.memories.selectAll')}
          </Checkbox>
          {selected.length > 0 && (
            <>
              <span className='text-12px text-t-secondary tabular-nums'>
                {t('nomi.memories.selectedCount', { count: selected.length })}
              </span>
              {memStatus === 'active' ? (
                <Button size='mini' onClick={() => confirmBatch('archive')}>
                  {t('nomi.memories.batchArchive')}
                </Button>
              ) : (
                <Button size='mini' onClick={() => confirmBatch('restore')}>
                  {t('nomi.memories.batchRestore')}
                </Button>
              )}
              <Button size='mini' onClick={() => setReclassifyVisible(true)}>
                {t('nomi.memories.batchReclassify')}
              </Button>
              <Button size='mini' status='danger' onClick={() => confirmBatch('delete')}>
                {t('nomi.memories.batchDelete')}
              </Button>
              <Button size='mini' type='text' onClick={() => setSelected([])}>
                {t('nomi.memories.clearSelection')}
              </Button>
            </>
          )}
        </div>
      )}

      {initialLoading ? (
        <div className='flex justify-center py-40px'>
          <Spin />
        </div>
      ) : memories.length === 0 ? (
        <Empty description={t('nomi.memories.empty')} />
      ) : (
        <div className='flex flex-col gap-8px transition-opacity duration-150' style={{ opacity: loading ? 0.6 : 1 }}>
          {memories.map((m) => (
            <div
              key={m.memory_id}
              className='group flex items-start gap-10px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-12px py-10px transition-colors hover:bg-fill-2'
            >
              <Checkbox
                className='mt-2px'
                checked={selected.includes(m.memory_id)}
                onChange={() => toggleSelected(m.memory_id)}
              />
              <Tag color={KIND_COLORS[m.kind]}>{t(`nomi.kinds.${m.kind}`)}</Tag>
              <div className='flex-1 min-w-0'>
                <div className='line-clamp-2 text-13px leading-20px text-t-primary break-words'>{memoryContent(m)}</div>
                <div className='mt-5px flex flex-wrap items-center gap-x-10px gap-y-4px text-11px text-t-tertiary'>
                  {scopeBadge(m)}
                  <span>
                    {t('nomi.memories.strength')} {(m.strength * 100).toFixed(0)}%
                  </span>
                  <span>{new Date(m.updated_at).toLocaleString()}</span>
                  {m.source !== 'learn' && <span>{t(`nomi.memories.source_${m.source}`, m.source)}</span>}
                </div>
              </div>
              <div className='flex items-center gap-4px shrink-0'>
                {m.status === 'archived' && (
                  <Button size='mini' onClick={() => void toggleArchive(m)}>
                    {t('nomi.memories.restore')}
                  </Button>
                )}
                <Tooltip content={m.pinned ? t('nomi.memories.unpin') : t('nomi.memories.pin')}>
                  <Button
                    size='mini'
                    type={m.pinned ? 'primary' : 'secondary'}
                    icon={<Pin theme='outline' size='12' />}
                    onClick={() => void togglePin(m)}
                  />
                </Tooltip>
                <Dropdown droplist={memoryActionMenu(m)} trigger='click' position='br' getPopupContainer={() => document.body}>
                  <Tooltip content={t('nomi.memories.more')}>
                    <Button size='mini' type='text' icon={<More theme='outline' size='14' />} aria-label={t('nomi.memories.more')} />
                  </Tooltip>
                </Dropdown>
              </div>
            </div>
          ))}
        </div>
      )}

      {total > 0 && (
        <div className='flex flex-wrap items-center justify-between gap-10px pt-2px'>
          <span className='text-12px text-t-tertiary tabular-nums'>{t('nomi.memories.total', { count: total })}</span>
          <Pagination
            current={page}
            pageSize={pageSize}
            total={total}
            showTotal
            sizeCanChange
            sizeOptions={[10, 20, 50]}
            showJumper={total > pageSize}
            onChange={handlePageChange}
          />
        </div>
      )}

      <Modal
        title={t('nomi.memories.reclassifyTitle')}
        visible={reclassifyVisible}
        onOk={() => void submitReclassify()}
        onCancel={() => setReclassifyVisible(false)}
      >
        <div className='flex flex-col gap-12px'>
          <div className='text-13px text-t-secondary'>{t('nomi.memories.reclassifyPick', { count: selected.length })}</div>
          <Select value={reclassifyKind} onChange={(v: ICompanionMemoryKind) => setReclassifyKind(v)}>
            {KINDS.map((k) => (
              <Select.Option key={k} value={k}>
                {t(`nomi.kinds.${k}`)}
              </Select.Option>
            ))}
          </Select>
        </div>
      </Modal>

      <Drawer
        width={520}
        title={t('nomi.memories.mergeTitle')}
        visible={mergeVisible}
        onCancel={() => setMergeVisible(false)}
        footer={null}
      >
        {mergeLoading ? (
          <div className='flex justify-center py-40px'>
            <Spin />
          </div>
        ) : mergeGroups.length === 0 ? (
          <Empty description={t('nomi.memories.mergeEmpty')} />
        ) : (
          <div className='flex flex-col gap-16px'>
            <div className='text-12px text-t-tertiary'>{t('nomi.memories.mergeHint')}</div>
            {mergeGroups.map((group, index) => {
              const draft = mergeDrafts[index];
              if (!draft) return null;
              const mergeInvalid = draft.ids.length < 2 || !draft.content.trim();
              return (
                <div
                  key={group.memories[0]?.memory_id ?? index}
                  className='flex flex-col gap-8px rounded-12px border border-solid border-[var(--color-border-2)] p-12px'
                >
                  {group.memories.map((m) => (
                    <Checkbox
                      key={m.memory_id}
                      checked={draft.ids.includes(m.memory_id)}
                      onChange={() =>
                        patchDraft(index, {
                          ids: draft.ids.includes(m.memory_id)
                            ? draft.ids.filter((id) => id !== m.memory_id)
                            : [...draft.ids, m.memory_id],
                        })
                      }
                    >
                      <span className='text-13px break-words'>{m.content}</span>
                    </Checkbox>
                  ))}
                  <div className='flex items-center gap-8px'>
                    <Select
                      size='small'
                      style={{ width: 140 }}
                      value={draft.kind}
                      onChange={(v: ICompanionMemoryKind) => patchDraft(index, { kind: v })}
                    >
                      {KINDS.map((k) => (
                        <Select.Option key={k} value={k}>
                          {t(`nomi.kinds.${k}`)}
                        </Select.Option>
                      ))}
                    </Select>
                    <span className='text-12px text-t-tertiary'>{t('nomi.memories.mergeContentLabel')}</span>
                  </div>
                  <Input.TextArea rows={3} value={draft.content} onChange={(v: string) => patchDraft(index, { content: v })} />
                  <div className='flex justify-end'>
                    <Button size='small' type='primary' disabled={mergeInvalid} onClick={() => void submitMerge(index)}>
                      {t('nomi.memories.mergeSubmit')}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Drawer>

      <Modal
        title={t('nomi.memories.add')}
        visible={addVisible}
        onOk={() => void add()}
        onCancel={() => setAddVisible(false)}
        okButtonProps={{ disabled: addInvalid }}
      >
        <div className='flex flex-col gap-12px'>
          <Select value={addKind} onChange={setAddKind}>
            {KINDS.map((k) => (
              <Select.Option key={k} value={k}>
                {t(`nomi.kinds.${k}`)}
              </Select.Option>
            ))}
          </Select>
          {scopeSelector(addScopeKind, addScopeCompanionId, setAddScopeKind, setAddScopeCompanionId)}
          <Input.TextArea
            rows={4}
            value={addContent}
            onChange={setAddContent}
            placeholder={t('nomi.memories.addPlaceholder')}
          />
        </div>
      </Modal>

      <Modal
        title={t('nomi.memories.edit')}
        visible={!!editTarget}
        onOk={() => void saveEdit()}
        onCancel={() => setEditTarget(null)}
        okButtonProps={{ disabled: editInvalid }}
      >
        <div className='flex flex-col gap-12px'>
          {scopeSelector(editScopeKind, editScopeCompanionId, setEditScopeKind, setEditScopeCompanionId)}
          <Input.TextArea rows={5} value={editContent} onChange={setEditContent} />
          <div className='text-11px text-t-tertiary'>{t('nomi.memories.editHint')}</div>
        </div>
      </Modal>

      <Modal
        title={t('nomi.memories.delete')}
        visible={!!deleteTarget}
        onOk={() => void confirmRemove()}
        onCancel={() => setDeleteTarget(null)}
        okButtonProps={{ status: 'danger' }}
      >
        {t('nomi.memories.deleteConfirm')}
      </Modal>
    </div>
  );
};

export default MemoriesTab;
