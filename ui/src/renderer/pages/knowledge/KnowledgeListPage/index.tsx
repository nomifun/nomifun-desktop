/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeListPage — Card grid with two-dimension filters (kind + tag).
 *
 * Consumes KnowledgeCard (B2), KnowledgeTagFilterBar (B3), useKnowledgeBases,
 * useKnowledgeTags (B1). Uses CreateStudio (Phase C) for the create flow.
 * The old Form-based create Modal has been removed; only the edit path (openEdit)
 * retains a simple modal.
 */
import React, { useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Button,
  Checkbox,
  Form,
  Input,
  Message,
  Modal,
  Result,
  Tooltip,
  Typography,
} from '@arco-design/web-react';
import { AddOne, Brain, Search, SettingTwo } from '@icon-park/react';
import { useLayoutContext } from '@renderer/hooks/context/LayoutContext';
import { isDesktopShell } from '@renderer/utils/platform';
import { ipcBridge } from '@/common';
import { HUB_PAGE_TITLE_CLASS } from '@/renderer/components/layout/HubPageShell';
import type { KnowledgeKindShortcut } from '../KnowledgeEmptyState';
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import {
  knowledgeErrorText,
  useKnowledgeBases,
} from '../useKnowledge';
import { useKnowledgeTags } from '../useKnowledgeTags';
import KnowledgeEmptyState from '../KnowledgeEmptyState';
import KnowledgeCard from '../KnowledgeCard';
import KnowledgeTagFilterBar, {
  type KnowledgeKind,
  type KnowledgeSort,
  type KnowledgeSortDirection,
} from '../KnowledgeTagFilterBar';
import toolbarStyles from '../KnowledgeTagFilterBar.module.css';
import { sortKnowledgeBases } from '../knowledgeSort';
import KnowledgeTagManagementModal from '../KnowledgeTagManagementModal';
import KnowledgeRetrievalSettingsModal from '../KnowledgeRetrievalSettingsModal';
import CreateStudio from '../CreateStudio';
import type { StudioInitialKind } from '../CreateStudio/sourceTypes';

// Keep the catalog compact and responsive: the existing 1180px page shell
// caps the grid at three 290px+ cards, while auto-fill steps down to two and
// one column only as the actual content area narrows. `min(..., 100%)` keeps
// the single-column layout overflow-safe on small screens.
const KNOWLEDGE_CARD_GRID_COLUMNS = 'repeat(auto-fill, minmax(min(290px, 100%), 1fr))';

// ─── Filter pure function ────────────────────────────────────────────────────

/**
 * Pure filter: kind dimension (exact match), tag dimension (OR / union within
 * selected tags), search dimension (name or description substring, case-insensitive).
 * Dimensions are AND-ed together.
 */
export function filterBases(
  bases: IKnowledgeBase[],
  kind: KnowledgeKind | 'all',
  tagKeys: string[],
  q: string
): IKnowledgeBase[] {
  const lq = q.toLowerCase().trim();
  return bases.filter(
    (b) =>
      (kind === 'all' || b.kind === kind) &&
      (tagKeys.length === 0 || tagKeys.some((k) => b.tags.includes(k))) &&
      (!lq || b.name.toLowerCase().includes(lq) || (b.description ?? '').toLowerCase().includes(lq))
  );
}

/** Self-managing checkbox for the imperative delete Modal.confirm. The modal
 * content never re-renders from page state, so this holds its own `checked`
 * state (so it toggles visually) and reports every change via `onChange`. */
const PurgeFilesCheckbox: React.FC<{ label: string; onChange: (v: boolean) => void }> = ({ label, onChange }) => {
  const [checked, setChecked] = useState(false);
  return (
    <Checkbox
      checked={checked}
      onChange={(v) => {
        setChecked(v);
        onChange(v);
      }}
    >
      {label}
    </Checkbox>
  );
};

// ─── Main Component ──────────────────────────────────────────────────────────

const KnowledgeListPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;

  // Data
  const { bases, loading, error, refresh } = useKnowledgeBases();
  const { tags, createTag, updateTag, deleteTag } = useKnowledgeTags();
  const [tagModalVisible, setTagModalVisible] = useState(false);
  const [retrievalModalVisible, setRetrievalModalVisible] = useState(false);

  // Filter state
  const [kindFilter, setKindFilter] = useState<KnowledgeKind | null>(null);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState('');
  const [sort, setSort] = useState<KnowledgeSort>('updated');
  const [sortDirection, setSortDirection] = useState<KnowledgeSortDirection>('desc');

  // Compute counts from the full (unfiltered) set
  const kindCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const b of bases) {
      counts[b.kind] = (counts[b.kind] ?? 0) + 1;
    }
    return counts;
  }, [bases]);

  const tagCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const b of bases) {
      for (const tk of b.tags) {
        counts[tk] = (counts[tk] ?? 0) + 1;
      }
    }
    return counts;
  }, [bases]);

  // Tag map for KnowledgeCard
  const tagMap = useMemo(() => {
    const m: Record<string, IKnowledgeTag> = {};
    for (const tag of tags) m[tag.key] = tag;
    return m;
  }, [tags]);

  // Filtered + sorted result
  const displayBases = useMemo(
    () => sortKnowledgeBases(filterBases(bases, kindFilter ?? 'all', tagFilter, searchQuery), sort, sortDirection),
    [bases, kindFilter, tagFilter, searchQuery, sort, sortDirection]
  );

  // ─── CreateStudio state ─────────────────────────────────────────────────────

  const [studioVisible, setStudioVisible] = useState(false);
  const [studioInitialKind, setStudioInitialKind] = useState<StudioInitialKind | undefined>(undefined);

  const openStudio = (initialKind?: KnowledgeKindShortcut) => {
    setStudioInitialKind(initialKind);
    setStudioVisible(true);
  };

  const handleStudioCreated = (base: unknown) => {
    setStudioVisible(false);
    void refresh();
    // Navigate to the new base detail
    if (base && typeof base === 'object' && 'knowledge_base_id' in base) {
      navigate(`/knowledge/${(base as IKnowledgeBase).knowledge_base_id}`);
    }
  };

  // ─── Edit Modal (lightweight — only for renaming/describing existing bases) ─

  const [form] = Form.useForm<{ name: string; description?: string }>();
  const [editing, setEditing] = useState<IKnowledgeBase | null>(null);
  const [editModalVisible, setEditModalVisible] = useState(false);
  const [saving, setSaving] = useState(false);

  const openEdit = (base: IKnowledgeBase) => {
    setEditing(base);
    form.resetFields();
    form.setFieldsValue({ name: base.name, description: base.description });
    setEditModalVisible(true);
  };

  const closeEditModal = () => {
    setEditModalVisible(false);
    setEditing(null);
  };

  const handleEditSubmit = async () => {
    try {
      const values = await form.validate();
      if (!editing) return;
      setSaving(true);
      await ipcBridge.knowledge.updateBase.invoke({
        knowledge_base_id: editing.knowledge_base_id,
        name: values.name,
        description: values.description ?? '',
      });
      Message.success(t('knowledge.actions.saveOk'));
      closeEditModal();
      void refresh();
    } catch (e) {
      if (e instanceof Error || typeof e === 'string') Message.error(knowledgeErrorText(e));
    } finally {
      setSaving(false);
    }
  };

  // ─── Delete ─────────────────────────────────────────────────────────────────

  // The delete confirm uses the imperative Modal.confirm, whose `content` is
  // rendered ONCE and never re-renders from page state — a page-level
  // `useState` checkbox could neither toggle visually nor be read by the
  // already-captured `onOk` closure. A ref carries the choice instead, and the
  // checkbox below manages its own checked state.
  const purgeRef = useRef(false);

  const handleDelete = async (base: IKnowledgeBase) => {
    try {
      await ipcBridge.knowledge.deleteBase.invoke({ knowledge_base_id: base.knowledge_base_id, purge: base.managed && purgeRef.current });
      Message.success(t('knowledge.actions.deleteOk'));
      purgeRef.current = false;
      void refresh();
    } catch (e) {
      Message.error(String(e));
    }
  };

  const handleCardDelete = (base: IKnowledgeBase, _e: React.MouseEvent) => {
    purgeRef.current = false;
    Modal.confirm({
      title: t('knowledge.actions.deleteConfirm', { defaultValue: '确认删除？' }),
      content: base.managed ? (
        <PurgeFilesCheckbox
          label={t('knowledge.actions.deleteWithFiles', { defaultValue: '同时删除文件' })}
          onChange={(v) => {
            purgeRef.current = v;
          }}
        />
      ) : undefined,
      onOk: () => handleDelete(base),
      onCancel: () => {
        purgeRef.current = false;
      },
    });
  };

  // ─── Import (direct for empty state) ──────────────────────────────────────

  const handleImport = async () => {
    try {
      if (isDesktopShell()) {
        const files = await ipcBridge.dialog.showOpen.invoke({
          properties: ['openFile'],
          filters: [{ name: 'Knowledge Base Archive', extensions: ['zip'] }],
        });
        if (!files?.[0]) return;
        await ipcBridge.knowledge.importBase.invoke({ src_path: files[0] });
      } else {
        Message.info(t('knowledge.empty.importDesktopOnly', { defaultValue: '导入功能仅桌面端可用' }));
        return;
      }
      Message.success(t('knowledge.empty.importOk', { defaultValue: '导入成功' }));
      void refresh();
    } catch (e) {
      Message.error(knowledgeErrorText(e));
    }
  };

  // ─── Tag management modal ─────────────────────────────────────────────────

  const handleManageTags = () => {
    setTagModalVisible(true);
  };

  // ─── Render ─────────────────────────────────────────────────────────────────

  const searchLabel = t('knowledge.searchPlaceholder', { defaultValue: '搜索知识库...' });
  const manageTagsLabel = t('knowledge.filter.manageTags', { defaultValue: '管理标签' });
  const retrievalSettingsLabel = t('knowledge.retrieval.open');
  const newBaseLabel = t('knowledge.newBase', { defaultValue: '新建知识库' });

  return (
    <div
      className={[
        'size-full box-border overflow-y-auto',
        isMobile ? 'px-16px py-14px' : 'px-12px py-24px md:px-40px md:py-32px',
      ].join(' ')}
    >
      <div className='mx-auto flex w-full max-w-1180px box-border flex-col gap-12px'>
        {/* Header */}
        <div className='w-full'>
          <h1 className={HUB_PAGE_TITLE_CLASS}>
            {t('knowledge.title', { defaultValue: '知识库' })}
          </h1>
          <Typography.Paragraph className='!m-0 !mt-6px max-w-1000px text-13px leading-20px text-[var(--color-text-3)]'>
            {t('knowledge.subtitle', { defaultValue: '集中管理你的专属领域知识。任意会话、终端、数字伙伴都能挂载它作为模型的扩展知识来源。' })}
          </Typography.Paragraph>
        </div>

        {/* Compact filter and action toolbar */}
        <KnowledgeTagFilterBar
          kindFilter={kindFilter}
          tagFilter={tagFilter}
          onKindChange={setKindFilter}
          onTagChange={setTagFilter}
          kindCounts={kindCounts}
          tagCounts={tagCounts}
          tags={tags}
          sort={sort}
          onSortChange={setSort}
          sortDirection={sortDirection}
          onSortDirectionChange={setSortDirection}
          actions={(
            <div
              className={[
                'flex min-w-0 items-center gap-6px',
                isMobile ? 'w-full' : 'flex-1 justify-end',
                !isMobile ? toolbarStyles.desktopActions : '',
              ].join(' ')}
            >
              {/* Search */}
              <Tooltip content={searchLabel} position='top' mini>
                <div
                  className={[
                    'flex h-34px box-border min-w-0 items-center gap-7px rounded-full px-11px',
                    'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                    'focus-within:border-primary-6 transition-colors',
                    isMobile ? 'flex-1' : 'w-220px',
                    !isMobile ? toolbarStyles.desktopSearch : '',
                  ].join(' ')}
                >
                  <span
                    className={[
                      'inline-flex h-18px w-18px flex-none items-center justify-center',
                      toolbarStyles.actionIcon,
                      !isMobile ? toolbarStyles.desktopSearchIcon : '',
                    ].join(' ')}
                  >
                    <Search theme='outline' size={14} className='block text-[var(--color-text-3)]' />
                  </span>
                  <input
                    aria-label={searchLabel}
                    className={[
                      'w-full border-none bg-transparent text-13px leading-18px text-[var(--color-text-1)] outline-none font-[inherit] placeholder:text-[var(--color-text-3)]',
                      !isMobile ? toolbarStyles.desktopSearchInput : '',
                    ].join(' ')}
                    placeholder={searchLabel}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                  />
                </div>
              </Tooltip>

              {/* Install-wide, task-exact knowledge retrieval models */}
              <Tooltip content={retrievalSettingsLabel} position='top' mini>
                <div
                  role='button'
                  tabIndex={0}
                  aria-label={retrievalSettingsLabel}
                  onClick={() => setRetrievalModalVisible(true)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      setRetrievalModalVisible(true);
                    }
                  }}
                  className={[
                    'inline-flex h-34px box-border flex-none items-center gap-6px rounded-full px-12px leading-none',
                    'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                    'text-13px font-medium text-[var(--color-text-1)] cursor-pointer select-none',
                    'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
                    'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
                    !isMobile ? toolbarStyles.desktopIconAction : '',
                  ].join(' ')}
                >
                  <span className={`${toolbarStyles.actionIcon} inline-flex h-18px w-18px flex-none items-center justify-center`}>
                    <Brain theme='outline' size={14} strokeWidth={3} className='block' />
                  </span>
                  {!isMobile && (
                    <span className={`${toolbarStyles.desktopActionLabel} inline-flex h-18px items-center leading-18px`}>
                      {retrievalSettingsLabel}
                    </span>
                  )}
                </div>
              </Tooltip>

              {/* Tag management */}
              <Tooltip content={manageTagsLabel} position='top' mini>
                <div
                  role='button'
                  tabIndex={0}
                  aria-label={manageTagsLabel}
                  onClick={handleManageTags}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      handleManageTags();
                    }
                  }}
                  className={[
                    'inline-flex h-34px box-border flex-none items-center gap-6px rounded-full px-12px leading-none',
                    'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
                    'text-13px font-medium text-[var(--color-text-1)] cursor-pointer select-none',
                    'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
                    'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
                    !isMobile ? toolbarStyles.desktopIconAction : '',
                  ].join(' ')}
                >
                  <span className={`${toolbarStyles.actionIcon} inline-flex h-18px w-18px flex-none items-center justify-center`}>
                    <SettingTwo theme='outline' size={14} strokeWidth={3} className='block' />
                  </span>
                  {!isMobile && (
                    <span className={`${toolbarStyles.desktopActionLabel} inline-flex h-18px items-center leading-18px`}>
                      {manageTagsLabel}
                    </span>
                  )}
                </div>
              </Tooltip>

              {/* Create button */}
              <Tooltip content={newBaseLabel} position='top' mini>
                <div
                  role='button'
                  tabIndex={0}
                  aria-label={newBaseLabel}
                  onClick={() => openStudio()}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      openStudio();
                    }
                  }}
                  className={[
                    'inline-flex h-34px box-border flex-none items-center gap-6px cursor-pointer select-none leading-none',
                    'rounded-full px-14px text-13px font-700',
                    'border border-solid border-transparent',
                    'bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)]',
                    'hover:bg-[rgba(var(--primary-6),0.18)]',
                    'focus-visible:border-primary-6 focus-visible:outline-none',
                    'transition-colors',
                    !isMobile ? toolbarStyles.desktopIconAction : '',
                  ].join(' ')}
                >
                  <span className={`${toolbarStyles.actionIcon} inline-flex h-18px w-18px flex-none items-center justify-center`}>
                    <AddOne theme='outline' size={15} strokeWidth={4} className='block text-primary-6' />
                  </span>
                  {!isMobile && (
                    <span className={`${toolbarStyles.desktopActionLabel} inline-flex h-18px items-center leading-18px`}>
                      {newBaseLabel}
                    </span>
                  )}
                </div>
              </Tooltip>
            </div>
          )}
        />

        {/* Error state */}
        {error ? (
          <Result
            status='error'
            title={t('knowledge.loadError', { defaultValue: '加载失败' })}
            subTitle={error}
            extra={<Button onClick={() => void refresh()}>{t('knowledge.retry', { defaultValue: '重试' })}</Button>}
          />
        ) : bases.length === 0 && !loading ? (
          <KnowledgeEmptyState onCreate={openStudio} onImport={() => void handleImport()} />
        ) : (
          <>
            {/* Card grid */}
            <div className='grid gap-16px' style={{ gridTemplateColumns: KNOWLEDGE_CARD_GRID_COLUMNS }}>
              {displayBases.map((base) => (
                <KnowledgeCard
                  key={base.knowledge_base_id}
                  base={base}
                  tagMap={tagMap}
                  onOpen={(b) => navigate(`/knowledge/${b.knowledge_base_id}`)}
                  onEdit={(b) => openEdit(b)}
                  onDelete={handleCardDelete}
                />
              ))}

              {/* Add-new dashed card (always last) */}
              <div
                role='button'
                tabIndex={0}
                onClick={() => openStudio()}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    openStudio();
                  }
                }}
                className={[
                  'flex flex-col items-center justify-center gap-8px cursor-pointer select-none',
                  'min-h-188px rounded-16px',
                  'border border-dashed border-[var(--color-border-3)] bg-transparent',
                  'text-[var(--color-text-3)]',
                  'hover:border-[var(--color-primary-light-3)] hover:text-primary-6 hover:bg-[var(--color-primary-light-1)]',
                  // This card is in the tab order (focusable div), and nothing
                  // styles a bare focusable div, so without these the keyboard
                  // focus was invisible. Mirrors the hover treatment;
                  // border-dashed is untouched because only the colour changes.
                  'focus-visible:border-primary-6 focus-visible:text-primary-6 focus-visible:outline-none',
                  'transition-all duration-150',
                ].join(' ')}
              >
                <div className='w-38px h-38px rounded-full border border-solid border-current grid place-items-center text-20px leading-none'>
                  ＋
                </div>
                <span className='text-13px'>{t('knowledge.newBase', { defaultValue: '新建知识库' })}</span>
              </div>
            </div>

            {/* Empty filter result */}
            {displayBases.length === 0 && bases.length > 0 && (
              <div className='flex flex-col items-center gap-8px py-40px text-[var(--color-text-3)] text-13px'>
                {t('knowledge.filterEmpty', { defaultValue: '没有匹配的知识库' })}
              </div>
            )}
          </>
        )}
      </div>

      {/* ─── CreateStudio (replaces old create Modal) ────────────────────────── */}
      <CreateStudio
        visible={studioVisible}
        initialKind={studioInitialKind}
        onClose={() => setStudioVisible(false)}
        onCreated={handleStudioCreated}
      />

      {/* ─── Edit Modal (lightweight, for existing bases only) ────────────────── */}
      <Modal
        title={t('knowledge.editBase')}
        visible={editModalVisible}
        confirmLoading={saving}
        onOk={() => void handleEditSubmit()}
        onCancel={closeEditModal}
        autoFocus={false}
      >
        <Form form={form} layout='vertical'>
          <Form.Item
            label={t('knowledge.form.name')}
            field='name'
            rules={[{ required: true, message: t('knowledge.form.nameRequired') }]}
          >
            <Input placeholder={t('knowledge.form.namePlaceholder')} maxLength={64} />
          </Form.Item>
          <Form.Item label={t('knowledge.form.description')} field='description'>
            <Input.TextArea
              placeholder={t('knowledge.form.descriptionPlaceholder')}
              autoSize={{ minRows: 2, maxRows: 4 }}
              maxLength={500}
            />
          </Form.Item>
        </Form>
      </Modal>

      {/* ─── Tag Management Modal ─────────────────────────────────────────── */}
      <KnowledgeTagManagementModal
        visible={tagModalVisible}
        onClose={() => setTagModalVisible(false)}
        tags={tags}
        createTag={createTag}
        updateTag={updateTag}
        deleteTag={deleteTag}
      />
      <KnowledgeRetrievalSettingsModal
        visible={retrievalModalVisible}
        onClose={() => setRetrievalModalVisible(false)}
      />
    </div>
  );
};

export default KnowledgeListPage;
