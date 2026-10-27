/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeListPage — Compact knowledge cards with kind, tag and search filters.
 * Creation uses CreateStudio; editing and deletion retain their existing dialogs.
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Button,
  Checkbox,
  Dropdown,
  Form,
  Input,
  Menu,
  Message,
  Modal,
  Tooltip,
} from '@arco-design/web-react';
import { Brain, Plus, Search, SettingTwo } from '@icon-park/react';
import { isDesktopShell } from '@renderer/utils/platform';
import { ipcBridge } from '@/common';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { preloadKnowledgeDetailRoute } from '@/renderer/components/layout/routePreload';
import type { KnowledgeKindShortcut } from '../KnowledgeEmptyState';
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import {
  prefetchKnowledgeBaseOpen,
  useKnowledgeBases,
} from '../useKnowledge';
import { useKnowledgeTags } from '../useKnowledgeTags';
import { KnowledgeErrorNotice, showKnowledgeErrorMessage } from '../KnowledgeErrorFeedback';
import KnowledgeEmptyState from '../KnowledgeEmptyState';
import KnowledgeCard from '../KnowledgeCard';
import FeishuKnowledgeCard from '../FeishuKnowledgeCard';
import KnowledgeTagFilterBar, {
  type KnowledgeKind,
  type KnowledgeSort,
  type KnowledgeSortDirection,
} from '../KnowledgeTagFilterBar';
import styles from './index.module.css';
import { sortKnowledgeBases } from '../knowledgeSort';
import KnowledgeTagManagementModal from '../KnowledgeTagManagementModal';
import KnowledgeRetrievalSettingsModal from '../KnowledgeRetrievalSettingsModal';
import CreateStudio from '../CreateStudio';
import type { StudioInitialKind } from '../CreateStudio/sourceTypes';

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

  // Data
  const { bases, loading, error, refresh } = useKnowledgeBases();
  const { tags, error: tagsError, refresh: refreshTags, createTag, updateTag, deleteTag } = useKnowledgeTags();
  const [tagModalVisible, setTagModalVisible] = useState(false);
  const [retrievalModalVisible, setRetrievalModalVisible] = useState(false);
  const [settingsMenuVisible, setSettingsMenuVisible] = useState(false);
  const openingBaseIdRef = useRef<string | null>(null);
  const [openingBaseId, setOpeningBaseId] = useState<string | null>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void preloadKnowledgeDetailRoute();
    }, 350);
    return () => window.clearTimeout(timer);
  }, []);

  const prefetchBase = useCallback((base: IKnowledgeBase) => {
    void preloadKnowledgeDetailRoute();
    void prefetchKnowledgeBaseOpen(base).catch(() => undefined);
  }, []);

  const openBase = useCallback(
    (base: IKnowledgeBase) => {
      if (openingBaseIdRef.current) return;
      openingBaseIdRef.current = base.knowledge_base_id;
      setOpeningBaseId(base.knowledge_base_id);
      prefetchBase(base);
      navigate(`/knowledge/${base.knowledge_base_id}`, {
        state: {
          routeLoadingLabel: t('knowledge.openingNamed', {
            name: base.name,
            defaultValue: '正在打开“{{name}}”…',
          }),
          knowledgeBase: base,
        },
      });
    },
    [navigate, prefetchBase, t]
  );

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
    // Feishu is a creatable base kind: the tile opens the studio pre-selected.
    // The pinned FeishuKnowledgeCard still routes to the connector workbench.
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
      // Arco validation errors are already rendered next to their fields.
      if (e instanceof Error || typeof e === 'string') showKnowledgeErrorMessage(e, 'operation', t);
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
      showKnowledgeErrorMessage(e, 'operation', t);
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
      showKnowledgeErrorMessage(e, 'operation', t);
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
    <div className={styles.page}>
      <div className={styles.content}>
        <PageHeader
          title={t('knowledge.title', { defaultValue: '知识库' })}
          description={t('knowledge.listDescription', { defaultValue: '管理文档与网页资料，在 Agent 会话中按需引用。' })}
          badge={<span className={styles.totalCount}>{bases.length}</span>}
          actions={(
            <>
              <Input
                className={styles.search}
                prefix={<Search theme='outline' size={14} strokeWidth={3} fill='currentColor' className={styles.actionIcon} />}
                aria-label={searchLabel}
                placeholder={searchLabel}
                value={searchQuery}
                onChange={setSearchQuery}
                allowClear
              />
              <Button
                type='primary'
                className={styles.createButton}
                icon={<Plus theme='outline' size={15} strokeWidth={3} fill='currentColor' className={styles.actionIcon} />}
                onClick={() => openStudio()}
              >
                {newBaseLabel}
              </Button>
              <Dropdown
                trigger='click'
                position='br'
                popupVisible={settingsMenuVisible}
                onVisibleChange={setSettingsMenuVisible}
                droplist={(
                  <Menu
                    onClickMenuItem={(key) => {
                      setSettingsMenuVisible(false);
                      if (key === 'retrieval') setRetrievalModalVisible(true);
                      else if (key === 'tags') handleManageTags();
                    }}
                  >
                    <Menu.Item key='retrieval'>
                      <span className={styles.settingsMenuItem}>
                        <Brain theme='outline' size={14} strokeWidth={3} fill='currentColor' className={styles.actionIcon} />
                        {retrievalSettingsLabel}
                      </span>
                    </Menu.Item>
                    <Menu.Item key='tags'>
                      <span className={styles.settingsMenuItem}>
                        <SettingTwo theme='outline' size={14} strokeWidth={3} fill='currentColor' className={styles.actionIcon} />
                        {manageTagsLabel}
                      </span>
                    </Menu.Item>
                  </Menu>
                )}
              >
                <Tooltip content={t('common.settings')} position='top' mini>
                  <Button
                    type='text'
                    className={styles.settingsButton}
                    icon={<SettingTwo theme='outline' size={16} strokeWidth={3} fill='currentColor' className={styles.actionIcon} />}
                    aria-label={t('common.settings')}
                    aria-haspopup='menu'
                    aria-expanded={settingsMenuVisible}
                  />
                </Tooltip>
              </Dropdown>
            </>
          )}
        />

        {bases.length > 0 && (
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
            resultCount={displayBases.length === bases.length
              ? t('knowledge.listCount', { count: bases.length, defaultValue: '{{count}} 个知识库' })
              : t('knowledge.filteredListCount', {
                visible: displayBases.length,
                total: bases.length,
                defaultValue: '{{visible}} / {{total}} 个知识库',
              })}
          />
        )}

        {/* Error state */}
        {error && <KnowledgeErrorNotice error={error} onRetry={() => void refresh()} />}
        {bases.length === 0 && !loading && !error ? (
          <KnowledgeEmptyState onCreate={openStudio} onImport={() => void handleImport()} />
        ) : bases.length > 0 || !error ? (
          <>
            {/* Card grid: the Feishu space entry is pinned only on the unfiltered view. */}
            <div className={styles.grid}>
              {!kindFilter && tagFilter.length === 0 && !searchQuery.trim() && <FeishuKnowledgeCard />}
              {displayBases.map((base) => (
                <KnowledgeCard
                  key={base.knowledge_base_id}
                  base={base}
                  tagMap={tagMap}
                  opening={openingBaseId === base.knowledge_base_id}
                  onPrefetch={prefetchBase}
                  onOpen={openBase}
                  onEdit={(b) => openEdit(b)}
                  onDelete={handleCardDelete}
                />
              ))}
            </div>

            {/* Empty filter result */}
            {displayBases.length === 0 && bases.length > 0 && (
              <div className={styles.filterEmpty} role='status'>
                {t('knowledge.filterEmpty', { defaultValue: '没有匹配的知识库' })}
              </div>
            )}
          </>
        ) : null}
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
        loadError={tagsError}
        onRetryLoad={() => void refreshTags()}
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
