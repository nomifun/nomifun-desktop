/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeDetailPage — Tab-shell redesign (Phase D).
 *
 * Structure:
 *   Header: back + kind icon + name + kind badge + tags + actions + meta row
 *   Tabs:   docs | use | set
 *
 * Each tab body is a placeholder for D2-D5 tasks.
 * Existing document logic is preserved inline under the "docs" tab.
 */

import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import { useTranslation } from 'react-i18next';
import {
  Button,
  Checkbox,
  Dropdown,
  Empty,
  Input,
  Menu,
  Message,
  Modal,
  Result,
  Spin,
  Tabs,
  Tooltip,
  Tree,
} from '@arco-design/web-react';
import {
  ExpandDown,
  ExpandUp,
  FileFocus,
  Delete,
  EditTwo,
  FolderOpen,
  FolderPlus,
  Left,
  LinkCloud,
  LinkOne,
  MagicHat,
  More,
  Plus,
  Refresh,
  Right,
  Search,
  SettingTwo,
} from '@icon-park/react';
import type {
  IKnowledgeAddContentResult,
  IKnowledgeBase,
  IKnowledgeTag,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import Markdown from '@renderer/components/Markdown';
import NomiInput from '@/renderer/components/base/NomiInput';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { useLayoutContext } from '@renderer/hooks/context/LayoutContext';
import { ipcBridge } from '@/common';
import {
  formatSize,
  getBaseSource,
  isAutogenNoProviderError,
  knowledgeErrorText,
  notifySourceFetchResult,
  useKnowledgeBase,
} from '../useKnowledge';
import { useKnowledgeTags } from '../useKnowledgeTags';
import KnowledgeModelSelector, { useKnowledgeAutogenModel } from '../KnowledgeModelSelector';
import KnowledgeConsumersSection from '../KnowledgeConsumersSection';
import TagPicker from '../CreateStudio/TagPicker';
import { getKindConfig, KindIcon, type KindConfig } from '../knowledgeKind';
import KnowledgeAddContentControl, {
  type KnowledgeAddContentControlHandle,
} from './KnowledgeAddContentControl';
import {
  buildKnowledgeSearchTree,
  isKnowledgePathWithin,
  knowledgeFolderPathChain,
  mergeKnowledgeTreeChildren,
  parentDirOfKnowledgePath,
  preserveKnowledgeTreeChildren,
  replaceKnowledgePathPrefix,
} from './treeModel';

// ─── Tab keys (maps to ?tab= query values) ─────────────────────────────────────

type TabKey = 'docs' | 'use' | 'set';
const ALL_TABS: TabKey[] = ['docs', 'use', 'set'];

// ─── Kind config (shared with KnowledgeCard via ../knowledgeKind) ──────────────

/** Kind icon in a rounded square (52px for detail header, bigger than card). */
function DetailKindIcon({ kind, config }: { kind: IKnowledgeBase['kind']; config: KindConfig }) {
  return <KindIcon kind={kind} config={config} size={22} containerClass='w-52px h-52px rounded-14px' />;
}

function collectKnowledgeDirKeys(nodes: IKnowledgeTreeEntry[]): string[] {
  const keys: string[] = [];
  const visit = (items: IKnowledgeTreeEntry[]) => {
    for (const item of items) {
      if (!item.is_dir) continue;
      keys.push(item.rel_path);
      if (item.children?.length) visit(item.children);
    }
  };
  visit(nodes);
  return keys;
}

const knowledgeDetailSoftActiveClass =
  'knowledge-detail-soft-active border border-solid border-[rgba(var(--primary-6),0.26)] bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)] shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.06)]';
const knowledgeDetailSegmentIdleClass =
  'border border-solid border-transparent text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]';
const knowledgeDetailSettingsLabelClass = 'block text-13px font-600 text-[var(--color-text-1)]';
const knowledgeDetailSettingsInputClass = 'knowledge-detail-settings-input';

type KnowledgeIconButtonProps = {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  loading?: boolean;
  tooltipPosition?: 'top' | 'bottom';
};

/** Compact icon action shared by the document rail header and its fixed footer. */
const KnowledgeIconButton: React.FC<KnowledgeIconButtonProps> = ({
  label,
  icon,
  onClick,
  loading = false,
  tooltipPosition = 'bottom',
}) => (
  <Tooltip content={label} position={tooltipPosition} mini>
    <Button
      type='text'
      size='mini'
      shape='circle'
      className='knowledge-doc-icon-button'
      icon={icon}
      loading={loading}
      aria-label={label}
      onClick={onClick}
    />
  </Tooltip>
);

// ─── Settings Tab (D5) ────────────────────────────────────────────────────────

interface SettingsTabProps {
  base: IKnowledgeBase;
  allTags: IKnowledgeTag[];
  createTag: (label: string) => Promise<IKnowledgeTag>;
  onRefresh: () => void;
}

const SettingsTab: React.FC<SettingsTabProps> = ({ base, allTags, createTag, onRefresh }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();

  // ─── Editable fields (local state, save on button click) ──────────────────
  const [editName, setEditName] = useState(base.name);
  const [editDesc, setEditDesc] = useState(base.description);
  const [editTags, setEditTags] = useState<string[]>(base.tags);
  const [saving, setSaving] = useState(false);

  // Sync local state when base changes from parent refresh
  useEffect(() => {
    setEditName(base.name);
    setEditDesc(base.description);
    setEditTags(base.tags);
  }, [base.name, base.description, base.tags]);

  const isDirty = editName !== base.name || editDesc !== base.description || JSON.stringify(editTags) !== JSON.stringify(base.tags);

  const handleSaveInfo = async () => {
    if (!isDirty) return;
    setSaving(true);
    try {
      await ipcBridge.knowledge.updateBase.invoke({
        knowledge_base_id: base.knowledge_base_id,
        name: editName.trim() || base.name,
        description: editDesc,
        tags: editTags,
      });
      Message.success(t('knowledge.detail.settings.saveOk', { defaultValue: '保存成功' }));
      onRefresh();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  // ─── Source actions (per kind) ────────────────────────────────────────────
  const [sourceLoading, setSourceLoading] = useState(false);

  const handleRefreshSource = async () => {
    if (sourceLoading) return;
    setSourceLoading(true);
    try {
      const summary = await ipcBridge.knowledge.refreshSource.invoke({ knowledge_base_id: base.knowledge_base_id });
      notifySourceFetchResult(t, summary, t('knowledge.source.refreshOk', { defaultValue: '刷新完成，获取 {{fetched}} 条', fetched: summary.fetched }));
      onRefresh();
    } catch (e) {
      Message.error(knowledgeErrorText(e));
    } finally {
      setSourceLoading(false);
    }
  };

  // ─── Danger zone: export ──────────────────────────────────────────────────
  const [exporting, setExporting] = useState(false);

  const handleExport = async () => {
    if (exporting) return;
    const dirs = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
    if (!dirs || dirs.length === 0) return;
    const destDir = dirs[0];
    setExporting(true);
    try {
      const { dest_path } = await ipcBridge.knowledge.exportBase.invoke({
        knowledge_base_id: base.knowledge_base_id,
        dest_path: destDir,
      });
      Message.success(t('knowledge.detail.settings.exportOk', { defaultValue: '已导出至 {{path}}', path: dest_path }));
    } catch (e) {
      Message.error(String(e));
    } finally {
      setExporting(false);
    }
  };

  // ─── Danger zone: delete ──────────────────────────────────────────────────
  const [deleteModalVisible, setDeleteModalVisible] = useState(false);
  const [purge, setPurge] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await ipcBridge.knowledge.deleteBase.invoke({ knowledge_base_id: base.knowledge_base_id, purge });
      Message.success(t('knowledge.detail.settings.deleteOk', { defaultValue: '已删除' }));
      navigate('/knowledge');
    } catch (e) {
      Message.error(String(e));
    } finally {
      setDeleting(false);
      setDeleteModalVisible(false);
    }
  };

  return (
    <div className='knowledge-settings-layout flex max-w-900px flex-col gap-18px'>
      {/* ─── Basic info: name / description / tags ─── */}
      <NomiSettingList>
        <NomiSettingRow
          title={t('knowledge.detail.settings.labelName', { defaultValue: '名称' })}
          description={t('knowledge.detail.settings.nameHint', {
            defaultValue: '知识库的名称，会显示在列表与挂载选择中。',
          })}
          controls={
            <NomiInput
              contentFit
              contentMaxWidth={320}
              value={editName}
              onChange={setEditName}
              placeholder={t('knowledge.detail.settings.namePlaceholder', { defaultValue: '知识库名称' })}
            />
          }
        />
      </NomiSettingList>

      <NomiSettingSection
        title={t('knowledge.detail.settings.labelDesc', { defaultValue: '描述' })}
        description={t('knowledge.detail.settings.descHint', {
          defaultValue: '向模型说明此知识库的内容与适用场景，帮助判断何时检索此库。',
        })}
      >
        <Input.TextArea
          value={editDesc}
          onChange={setEditDesc}
          autoSize={{ minRows: 3, maxRows: 10 }}
          className={`${knowledgeDetailSettingsInputClass} knowledge-settings-description-input`}
          placeholder={t('knowledge.detail.settings.descPlaceholder', { defaultValue: '简要描述知识库内容和用途' })}
        />
      </NomiSettingSection>

      <section className='knowledge-settings-tags-section flex flex-col gap-8px'>
        <label className={knowledgeDetailSettingsLabelClass}>
          {t('knowledge.detail.settings.labelTags', { defaultValue: '标签' })}
        </label>
        <div className='knowledge-settings-tag-picker'>
          <TagPicker value={editTags} onChange={setEditTags} tags={allTags} createTag={createTag} />
        </div>
        <div className='mt-8px'>
          <Button
            className='knowledge-settings-save-button'
            type='primary'
            loading={saving}
            disabled={!isDirty}
            onClick={() => void handleSaveInfo()}
          >
            {t('knowledge.detail.settings.save', { defaultValue: '保存修改' })}
          </Button>
        </div>
      </section>

      {/* ─── Source section (varies by kind) ─── */}
      <NomiSettingList>
        <NomiSettingRow
          className='knowledge-settings-source-row'
          title={
            <>
              {t('knowledge.detail.settings.labelSource', { defaultValue: '来源' })}
              {' · '}
              {base.kind === 'local' && t('knowledge.card.kindLocal', { defaultValue: '本地文件夹' })}
              {base.kind === 'web' && t('knowledge.card.kindWeb', { defaultValue: '网页' })}
              {base.kind === 'blank' && t('knowledge.card.kindBlank', { defaultValue: '空白' })}
            </>
          }
          description={
            base.kind === 'web'
              ? t('knowledge.detail.settings.webHint', { defaultValue: '网页来源 — 点击“刷新”重新抓取所有 URL。' })
              : undefined
          }
          controls={
            base.kind === 'local' ? (
              <>
                <NomiInput contentFit contentMinWidth={220} contentMaxWidth={520} value={base.root_path} readOnly />
                <Button
                  icon={<FolderOpen theme='outline' size='14' />}
                  onClick={() => {
                    void ipcBridge.shell.openFolderWith
                      .invoke({ folder_path: base.root_path, tool: 'explorer' })
                      .catch((e: unknown) => Message.error(String(e)));
                  }}
                >
                  {t('knowledge.detail.settings.openFolder', { defaultValue: '打开' })}
                </Button>
              </>
            ) : base.kind === 'web' ? (
              <Button
                icon={<Refresh theme='outline' size='14' />}
                loading={sourceLoading}
                onClick={() => void handleRefreshSource()}
              >
                {t('knowledge.detail.settings.refreshSource', { defaultValue: '刷新' })}
              </Button>
            ) : undefined
          }
        />
      </NomiSettingList>

      {/* ─── Danger zone ─── */}
      <NomiSettingSection
        className='knowledge-settings-danger-section'
        title={t('knowledge.detail.settings.dangerTitle', { defaultValue: '危险操作' })}
      >
        <NomiSettingList>
          <NomiSettingRow
            title={t('knowledge.detail.settings.exportDesc', { defaultValue: '导出为 .zip 备份包' })}
            controls={
              <Button size='mini' loading={exporting} onClick={() => void handleExport()}>
                {t('knowledge.detail.settings.exportBtn', { defaultValue: '导出' })}
              </Button>
            }
          />
          <NomiSettingRow
            title={t('knowledge.detail.settings.deleteDesc', { defaultValue: '删除此知识库' })}
            description={
              !base.managed
                ? t('knowledge.detail.settings.deleteLocalHint', { defaultValue: '（本地引用目录不会被删除）' })
                : undefined
            }
            controls={
              <Button size='mini' status='danger' onClick={() => setDeleteModalVisible(true)}>
                {t('knowledge.detail.settings.deleteBtn', { defaultValue: '删除知识库' })}
              </Button>
            }
          />
        </NomiSettingList>
      </NomiSettingSection>

      {/* Delete confirmation modal */}
      <Modal
        title={t('knowledge.detail.settings.deleteModalTitle', { defaultValue: '确认删除知识库' })}
        visible={deleteModalVisible}
        onCancel={() => setDeleteModalVisible(false)}
        onOk={() => void handleDelete()}
        confirmLoading={deleting}
        okButtonProps={{ status: 'danger' }}
        okText={t('knowledge.detail.settings.deleteConfirm', { defaultValue: '确认删除' })}
      >
        <p className='text-13px text-[var(--color-text-2)] mb-12px'>
          {t('knowledge.detail.settings.deleteWarning', {
            defaultValue: '删除后无法恢复。知识库的所有文档、挂载关系将被清除。',
          })}
        </p>
        {base.managed && (
          <Checkbox checked={purge} onChange={setPurge}>
            <span className='text-12px text-[var(--color-text-3)]'>
              {t('knowledge.detail.settings.purgeOption', { defaultValue: '同时删除磁盘上的数据目录' })}
            </span>
          </Checkbox>
        )}
        {!base.managed && (
          <p className='text-12px text-[var(--color-text-3)] m-0 mt-8px'>
            {t('knowledge.detail.settings.deleteLocalNote', {
              defaultValue: '本知识库引用的外部目录（{{path}}）不会被删除，仅取消关联。',
              path: base.root_path,
            })}
          </p>
        )}
      </Modal>
    </div>
  );
};

// ─── Main Component ─────────────────────────────────────────────────────────────

const KnowledgeDetailPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id: rawId } = useParams<{ id: string }>();
  const id = rawId == null ? undefined : parseKnowledgeBaseId(rawId);
  const [searchParams, setSearchParams] = useSearchParams();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;

  // ─── Data hooks ─────────────────────────────────────────────────────────────
  const { base, files, tree, loading, error, refresh } = useKnowledgeBase(id);
  const { choice: modelChoice, setChoice: setModelChoice } = useKnowledgeAutogenModel();
  const { tags: allTags, createTag } = useKnowledgeTags();

  // ─── Tab routing via ?tab= ──────────────────────────────────────────────────
  const rawTabParam = searchParams.get('tab');
  const activeTab: TabKey = rawTabParam && ALL_TABS.includes(rawTabParam as TabKey) ? (rawTabParam as TabKey) : 'docs';

  const setTab = useCallback(
    (key: string) => {
      setSearchParams(
        (prev) => {
          prev.set('tab', key);
          return prev;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  // ─── Tag resolution ─────────────────────────────────────────────────────────
  const tagMap = useMemo(() => {
    const m: Record<string, IKnowledgeTag> = {};
    for (const tag of allTags) m[tag.key] = tag;
    return m;
  }, [allTags]);

  // ─── Document state (preserved from original — D2 will own this) ────────────
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [content, setContent] = useState<string>('');
  const [fileLoading, setFileLoading] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const [newFolderVisible, setNewFolderVisible] = useState(false);
  const [newFolderPath, setNewFolderPath] = useState('');
  const [renameVisible, setRenameVisible] = useState(false);
  const [renameTarget, setRenameTarget] = useState<IKnowledgeTreeEntry | null>(null);
  const [renameName, setRenameName] = useState('');
  const [autogenLoading, setAutogenLoading] = useState(false);
  const [refreshingSource, setRefreshingSource] = useState(false);
  const [treeData, setTreeData] = useState<IKnowledgeTreeEntry[]>([]);
  const [expandedTreeKeys, setExpandedTreeKeys] = useState<string[]>([]);
  const [selectedFolderPath, setSelectedFolderPath] = useState('');
  const [selectedTreeKey, setSelectedTreeKey] = useState<string | null>(null);
  const [fileSearch, setFileSearch] = useState('');
  const [treeAction, setTreeAction] = useState<'reveal' | 'expand' | null>(null);
  const treeScrollRef = React.useRef<HTMLDivElement>(null);
  const addContentControlRef = React.useRef<KnowledgeAddContentControlHandle>(null);
  const isTreeSearch = fileSearch.trim().length > 0;

  const source = getBaseSource(base);

  useEffect(() => {
    setTreeData((prev) => preserveKnowledgeTreeChildren(tree, prev));
  }, [tree]);

  // Auto-select first file
  useEffect(() => {
    if (!selectedPath && files.length > 0) {
      setSelectedPath(files[0].rel_path);
      setSelectedTreeKey(files[0].rel_path);
    }
    if (selectedPath && !files.some((f) => f.rel_path === selectedPath)) {
      const nextPath = files.length > 0 ? files[0].rel_path : null;
      setSelectedPath(nextPath);
      setSelectedTreeKey(nextPath);
    }
  }, [files, selectedPath]);

  // Reset per-base view state when switching knowledge bases — the route param
  // changes but React reuses this component instance, so the previous base's
  // document search query / edit mode would otherwise leak into the next base
  // (looking like "documents missing"). selectedPath is reconciled above.
  useEffect(() => {
    setFileSearch('');
    setEditMode(false);
    setExpandedTreeKeys([]);
    setSelectedFolderPath('');
    setSelectedTreeKey(null);
  }, [id]);

  // Load file content
  useEffect(() => {
    if (!id || !selectedPath) {
      setContent('');
      return;
    }
    let cancelled = false;
    setFileLoading(true);
    setEditMode(false);
    ipcBridge.knowledge.readFile
      .invoke({ knowledge_base_id: id, path: selectedPath })
      .then((res) => {
        if (!cancelled) setContent(res.content);
      })
      .catch((e) => {
        if (!cancelled) Message.error(String(e));
      })
      .finally(() => {
        if (!cancelled) setFileLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [id, selectedPath]);

  const startEdit = () => {
    setDraft(content);
    setEditMode(true);
  };

  const handleSave = async () => {
    if (!id || !selectedPath) return;
    setSaving(true);
    try {
      await ipcBridge.knowledge.writeFile.invoke({ knowledge_base_id: id, path: selectedPath, content: draft });
      setContent(draft);
      setEditMode(false);
      Message.success(t('knowledge.actions.saveOk'));
      void refresh();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleLoadTreeChildren = useCallback(
    async (node: IKnowledgeTreeEntry) => {
      if (!id || node.is_file || isTreeSearch) return;
      const children = await ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id, path: node.rel_path });
      setTreeData((prev) => mergeKnowledgeTreeChildren(prev, node.rel_path, children));
    },
    [id, isTreeSearch]
  );

  const reloadTreePath = useCallback(
    async (folderPath: string) => {
      if (!id) return;
      const rootChildren = await ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id });
      setTreeData(rootChildren);

      const branchesToReload = knowledgeFolderPathChain(folderPath);
      for (const branchPath of branchesToReload) {
        const children = await ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id, path: branchPath });
        setTreeData((prev) => mergeKnowledgeTreeChildren(prev, branchPath, children));
      }
      if (branchesToReload.length > 0) {
        setExpandedTreeKeys((prev) => [...new Set([...prev, ...branchesToReload])]);
      }
    },
    [id]
  );

  const scrollCurrentTreeNodeIntoView = useCallback(() => {
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        const container = treeScrollRef.current;
        const selectedNode = container?.querySelector<HTMLElement>('.arco-tree-node-selected');
        if (!container || !selectedNode) return;

        const containerRect = container.getBoundingClientRect();
        const nodeRect = selectedNode.getBoundingClientRect();
        const centeredTop =
          container.scrollTop + nodeRect.top - containerRect.top - (container.clientHeight - nodeRect.height) / 2;
        container.scrollTo({ top: Math.max(0, centeredTop), behavior: 'smooth' });
      });
    });
  }, []);

  const handleRevealCurrentFile = useCallback(async () => {
    if (!selectedPath) {
      Message.info(t('knowledge.selectFile'));
      return;
    }

    setTreeAction('reveal');
    try {
      const parentPath = parentDirOfKnowledgePath(selectedPath);
      const ancestorKeys = knowledgeFolderPathChain(parentPath);
      setFileSearch('');
      await reloadTreePath(parentPath);
      setExpandedTreeKeys((prev) => [...new Set([...prev, ...ancestorKeys])]);
      setSelectedTreeKey(selectedPath);
      scrollCurrentTreeNodeIntoView();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setTreeAction(null);
    }
  }, [reloadTreePath, scrollCurrentTreeNodeIntoView, selectedPath, t]);

  const handleExpandAllTreeNodes = useCallback(async () => {
    if (!id) return;

    setTreeAction('expand');
    try {
      const loadAllChildren = async (nodes: IKnowledgeTreeEntry[]): Promise<IKnowledgeTreeEntry[]> =>
        Promise.all(
          nodes.map(async (node) => {
            if (!node.is_dir) return node;
            const children = await ipcBridge.knowledge.listTree.invoke({
              knowledge_base_id: id,
              path: node.rel_path,
            });
            return { ...node, children: await loadAllChildren(children) };
          })
        );

      const rootNodes = await ipcBridge.knowledge.listTree.invoke({ knowledge_base_id: id });
      const fullTree = await loadAllChildren(rootNodes);
      setFileSearch('');
      setTreeData(fullTree);
      setExpandedTreeKeys(collectKnowledgeDirKeys(fullTree));
    } catch (e) {
      Message.error(String(e));
    } finally {
      setTreeAction(null);
    }
  }, [id]);

  const openNewFolderModal = (folderOverride?: string) => {
    const folder = folderOverride ?? (selectedFolderPath || parentDirOfKnowledgePath(selectedPath));
    setNewFolderPath(folder ? `${folder}/` : '');
    setNewFolderVisible(true);
  };

  const handleContentAdded = async (result: IKnowledgeAddContentResult) => {
    setFileSearch('');
    await refresh();
    if (result.type === 'document') {
      const parent = parentDirOfKnowledgePath(result.path);
      await reloadTreePath(parent);
      setSelectedPath(result.path);
      setSelectedTreeKey(result.path);
      setSelectedFolderPath(parent);
      return;
    }
    if (result.type === 'local_folder') {
      await reloadTreePath(result.target_directory);
      if (result.first_file) {
        setSelectedPath(result.first_file);
        setSelectedTreeKey(result.first_file);
        setSelectedFolderPath(parentDirOfKnowledgePath(result.first_file));
      } else {
        setSelectedFolderPath(result.target_directory);
        setSelectedTreeKey(result.target_directory);
      }
      return;
    }
    if (result.first_file) {
      const parent = parentDirOfKnowledgePath(result.first_file);
      await reloadTreePath(parent);
      setSelectedPath(result.first_file);
      setSelectedTreeKey(result.first_file);
      setSelectedFolderPath(parent);
    }
  };

  const openRenameModal = (item: IKnowledgeTreeEntry) => {
    setRenameTarget(item);
    setRenameName(item.name);
    setRenameVisible(true);
  };

  const handleCreateFolder = async () => {
    if (!id) return;
    const path = newFolderPath.trim().replace(/\\/g, '/').replace(/^\/+|\/+$/g, '');
    if (!path) return;
    const parent = parentDirOfKnowledgePath(path);
    try {
      await ipcBridge.knowledge.createFolder.invoke({ knowledge_base_id: id, path });
      setNewFolderVisible(false);
      setNewFolderPath('');
      setFileSearch('');
      await reloadTreePath(parent);
      setSelectedFolderPath(path);
      setSelectedTreeKey(path);
      Message.success(t('knowledge.actions.createFolderOk', { defaultValue: '文件夹已创建' }));
    } catch (e) {
      Message.error(String(e));
    }
  };

  const handleRenameTreeEntry = async () => {
    if (!id || !renameTarget) return;
    let newName = renameName.trim();
    if (!newName) return;
    if (renameTarget.is_file && !newName.toLowerCase().endsWith('.md')) newName = `${newName}.md`;
    const oldPath = renameTarget.rel_path;
    const parent = parentDirOfKnowledgePath(oldPath);
    try {
      const renamed = await ipcBridge.knowledge.renameTreeEntry.invoke({ knowledge_base_id: id, path: oldPath, newName });
      setRenameVisible(false);
      setRenameTarget(null);
      setRenameName('');
      setFileSearch('');
      setExpandedTreeKeys((prev) =>
        prev.map((key) => replaceKnowledgePathPrefix(key, oldPath, renamed.rel_path) ?? key)
      );
      setSelectedPath((prev) => replaceKnowledgePathPrefix(prev, oldPath, renamed.rel_path));
      setSelectedFolderPath((prev) => replaceKnowledgePathPrefix(prev || null, oldPath, renamed.rel_path) || '');
      setSelectedTreeKey((prev) => replaceKnowledgePathPrefix(prev, oldPath, renamed.rel_path));
      await refresh();
      await reloadTreePath(parent);
      Message.success(t('knowledge.actions.renameOk', { defaultValue: '已重命名' }));
    } catch (e) {
      Message.error(String(e));
    }
  };

  const handleDeleteFile = async (path: string) => {
    if (!id) return;
    const parent = parentDirOfKnowledgePath(path);
    try {
      await ipcBridge.knowledge.deleteFile.invoke({ knowledge_base_id: id, path });
      Message.success(t('knowledge.actions.deleteOk'));
      if (selectedPath === path) {
        setSelectedPath(null);
        setSelectedTreeKey(parent || null);
      }
      await refresh();
      await reloadTreePath(parent);
    } catch (e) {
      Message.error(String(e));
    }
  };

  const handleDeleteFolder = async (path: string) => {
    if (!id) return;
    const parent = parentDirOfKnowledgePath(path);
    try {
      await ipcBridge.knowledge.deleteFolder.invoke({ knowledge_base_id: id, path });
      Message.success(t('knowledge.actions.deleteFolderOk', { defaultValue: '目录已删除' }));
      setFileSearch('');
      setExpandedTreeKeys((prev) => prev.filter((key) => !isKnowledgePathWithin(key, path)));
      if (isKnowledgePathWithin(selectedPath, path)) {
        setSelectedPath(null);
        setContent('');
        setDraft('');
        setEditMode(false);
      }
      if (isKnowledgePathWithin(selectedTreeKey, path)) {
        setSelectedTreeKey(parent || null);
      }
      if (isKnowledgePathWithin(selectedFolderPath || null, path)) {
        setSelectedFolderPath(parent);
      }
      await refresh();
      await reloadTreePath(parent);
    } catch (e) {
      Message.error(String(e));
    }
  };

  const confirmDeleteTreeEntry = (item: IKnowledgeTreeEntry) => {
    if (item.is_dir) {
      Modal.confirm({
        title: t('knowledge.tree.deleteFolderTitle', { defaultValue: '确认删除目录？' }),
        content: (
          <div className='text-13px leading-20px text-[var(--color-text-2)]'>
            <div>
              {t('knowledge.tree.deleteFolderWarning', {
                defaultValue: '删除目录“{{name}}”会一并清空其下所有文档和子目录，无法撤销。',
                name: item.name,
              })}
            </div>
            <div className='mt-6px break-all text-[var(--color-text-3)]'>{item.rel_path}</div>
          </div>
        ),
        okButtonProps: { status: 'danger' },
        okText: t('knowledge.actions.delete', { defaultValue: '删除' }),
        onOk: () => handleDeleteFolder(item.rel_path),
      });
      return;
    }

    Modal.confirm({
      title: t('knowledge.actions.deleteFileConfirm', { defaultValue: '确认删除该文档？' }),
      content: <div className='break-all text-[var(--color-text-3)]'>{item.rel_path}</div>,
      okButtonProps: { status: 'danger' },
      okText: t('knowledge.actions.delete', { defaultValue: '删除' }),
      onOk: () => handleDeleteFile(item.rel_path),
    });
  };

  const handleTreeNodeMenuClick = (key: string, item: IKnowledgeTreeEntry) => {
    if (key === 'new-file' && item.is_dir) {
      addContentControlRef.current?.openDocument(item.rel_path);
      return;
    }
    if (key === 'new-folder' && item.is_dir) {
      openNewFolderModal(item.rel_path);
      return;
    }
    if (key === 'rename') {
      openRenameModal(item);
      return;
    }
    if (key === 'delete') {
      confirmDeleteTreeEntry(item);
    }
  };

  const handleOpenFolder = async () => {
    if (!base) return;
    try {
      await ipcBridge.shell.openFolderWith.invoke({ folder_path: base.root_path, tool: 'explorer' });
    } catch (e) {
      Message.error(String(e));
    }
  };

  const handleAutogen = async () => {
    if (!id || autogenLoading) return;
    setAutogenLoading(true);
    try {
      const res = await ipcBridge.knowledge.autogenBase.invoke({ knowledge_base_id: id, ...(modelChoice ?? {}) });
      Message.success(
        t(res.readme_written ? 'knowledge.actions.autogenOkReadme' : 'knowledge.actions.autogenOkNoReadme')
      );
      void refresh();
    } catch (e) {
      Message.error(isAutogenNoProviderError(e) ? t('knowledge.actions.autogenNoProvider') : knowledgeErrorText(e));
    } finally {
      setAutogenLoading(false);
    }
  };

  const handleRefreshSource = async () => {
    if (!id || refreshingSource) return;
    setRefreshingSource(true);
    try {
      const summary = await ipcBridge.knowledge.refreshSource.invoke({ knowledge_base_id: id });
      notifySourceFetchResult(t, summary, t('knowledge.source.refreshOk', { fetched: summary.fetched }));
      void refresh();
    } catch (e) {
      Message.error(knowledgeErrorText(e));
    } finally {
      setRefreshingSource(false);
    }
  };

  // ─── Computed ───────────────────────────────────────────────────────────────
  const kindConfig = base ? getKindConfig(base.kind, t, 'neutral') : null;

  const displayedTreeData = useMemo(
    () => (isTreeSearch ? buildKnowledgeSearchTree(files, fileSearch) : treeData),
    [files, fileSearch, isTreeSearch, treeData]
  );
  const visibleTreeExpandedKeys = useMemo(
    () => (isTreeSearch ? collectKnowledgeDirKeys(displayedTreeData) : expandedTreeKeys),
    [displayedTreeData, expandedTreeKeys, isTreeSearch]
  );
  const loadedTreeDirectoryKeys = useMemo(() => collectKnowledgeDirKeys(treeData), [treeData]);
  const isEntireTreeExpanded = useMemo(
    () =>
      loadedTreeDirectoryKeys.length > 0 &&
      loadedTreeDirectoryKeys.every((key) => expandedTreeKeys.includes(key)),
    [expandedTreeKeys, loadedTreeDirectoryKeys]
  );

  const handleToggleEntireTree = useCallback(() => {
    if (isEntireTreeExpanded && !isTreeSearch) {
      setExpandedTreeKeys([]);
      return;
    }
    void handleExpandAllTreeNodes();
  }, [handleExpandAllTreeNodes, isEntireTreeExpanded, isTreeSearch]);

  // Build breadcrumb segments from selected path
  const breadcrumbSegments = useMemo(() => {
    if (!selectedPath) return [];
    return selectedPath.split('/');
  }, [selectedPath]);

  const relativeTime = useMemo(() => {
    if (!base?.updated_at) return '';
    // updated_at is already epoch-MILLIS (TimestampMs / now_ms() on the backend);
    // KnowledgeCard's formatRelativeTime treats it as ms directly. The stray
    // `* 1000` here pushed it ~1.7e15, making diffMin always < 1 → forever "刚刚".
    const diffMs = Date.now() - base.updated_at;
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return t('knowledge.detail.justNow', { defaultValue: '刚刚' });
    if (diffMin < 60) return t('knowledge.detail.minutesAgo', { defaultValue: '{{n}} 分钟前', n: diffMin });
    const diffH = Math.floor(diffMin / 60);
    if (diffH < 24) return t('knowledge.detail.hoursAgo', { defaultValue: '{{n}} 小时前', n: diffH });
    const diffD = Math.floor(diffH / 24);
    return t('knowledge.detail.daysAgo', { defaultValue: '{{n}} 天前', n: diffD });
  }, [base?.updated_at, t]);

  // ─── Error state ────────────────────────────────────────────────────────────
  if (error) {
    return (
      <div className='size-full flex items-center justify-center'>
        <Result
          status='error'
          title={t('knowledge.loadError')}
          subTitle={error}
          extra={<Button onClick={() => navigate('/knowledge')}>{t('knowledge.backToList')}</Button>}
        />
      </div>
    );
  }

  // ─── Render ─────────────────────────────────────────────────────────────────
  return (
    <div
      className={classNames(
        'size-full box-border overflow-y-auto',
        isMobile ? 'px-16px py-14px' : 'px-12px py-24px md:px-40px md:py-32px'
      )}
    >
      <div className='mx-auto flex w-full max-w-1180px box-border flex-col gap-16px'>
        {/* ─── Back link ─────────────────────────────────────────────────────── */}
        <button
          type='button'
          className='knowledge-detail-back-link inline-flex h-24px items-center gap-6px border-0 bg-transparent p-0 font-[inherit] text-12px leading-none text-[var(--color-text-3)] appearance-none cursor-pointer transition-colors hover:text-primary-6 focus-visible:outline-none focus-visible:text-primary-6'
          onClick={() => navigate('/knowledge')}
        >
          <span className='knowledge-detail-back-icon inline-flex h-14px w-14px items-center justify-center leading-none [&_svg]:block'>
            <Left theme='outline' size='14' />
          </span>
          <span className='leading-none'>{t('knowledge.detail.back', { defaultValue: '返回知识库' })}</span>
        </button>

        {/* ─── Header ────────────────────────────────────────────────────────── */}
        <div className='flex flex-wrap items-start justify-between gap-18px'>
          {/* Left: icon + title + badges + tags */}
          <div className='flex gap-14px items-center'>
            {base && kindConfig && <DetailKindIcon kind={base.kind} config={kindConfig} />}
            <div className='flex flex-col gap-6px'>
              <h1 className='m-0 text-21px font-700 text-[var(--color-text-1)] flex items-center gap-9px'>
                {base?.name ?? '...'}
                {/* Pen icon — edit entry point (actual editing in D5/Settings tab) */}
                <span
                  className='text-12px text-[var(--color-text-3)] cursor-pointer hover:text-primary-6'
                  onClick={() => setTab('set')}
                  title={t('knowledge.detail.editName', { defaultValue: '编辑名称' })}
                >
                  <EditTwo theme='outline' size='12' />
                </span>
              </h1>
              <div className='flex flex-wrap items-center gap-6px'>
                {/* Kind badge */}
                {kindConfig && (
                  <span
                    className={`knowledge-detail-kind-badge inline-flex items-center rounded-6px px-8px py-2px text-10px font-600 border border-solid ${kindConfig.bgClass} ${kindConfig.textClass} ${kindConfig.borderClass}`}
                  >
                    {kindConfig.label}
                  </span>
                )}
                {/* User tags */}
                {base?.tags.map((tagKey) => {
                  const tag = tagMap[tagKey];
                  return (
                    <span
                      key={tagKey}
                      className='knowledge-detail-user-tag inline-flex items-center gap-5px text-11px font-500 text-[var(--color-text-1)] bg-[var(--color-fill-2)] border border-solid border-[var(--color-border-3)] rounded-6px px-8px py-2px'
                    >
                      {tag?.color && (
                        <i className='w-6px h-6px rounded-full inline-block' style={{ background: tag.color }} />
                      )}
                      {tag?.label ?? tagKey}
                    </span>
                  );
                })}
                {/* Add tag placeholder (leads to settings tab) */}
                <span
                  className='knowledge-detail-add-tag text-11px font-500 text-[var(--color-text-2)] bg-[var(--color-fill-1)] cursor-pointer border border-dashed border-[var(--color-border-3)] rounded-6px px-8px py-2px transition-colors hover:bg-[rgba(var(--primary-6),0.1)] hover:text-[var(--color-text-1)] hover:border-[rgba(var(--primary-6),0.36)]'
                  onClick={() => setTab('set')}
                >
                  + {t('knowledge.detail.addTag', { defaultValue: '标签' })}
                </span>
              </div>
            </div>
          </div>

          {/* Right: action buttons */}
          <div className='flex items-center gap-8px flex-wrap'>
            <Button
              shape='round'
              icon={<Search theme='outline' size='14' />}
              onClick={() => Message.info(t('knowledge.detail.searchPlaceholder', { defaultValue: '检索功能开发中' }))}
            >
              {t('knowledge.detail.search', { defaultValue: '检索' })}
            </Button>
            <Button
              type='primary'
              shape='round'
              icon={<LinkOne theme='outline' size='14' />}
              onClick={() => setTab('use')}
            >
              {t('knowledge.detail.mountToSession', { defaultValue: '挂载到会话' })}
            </Button>
            <Dropdown
              droplist={
                <Menu>
                  <Menu.Item key='export' onClick={() => setTab('set')}>
                    {t('knowledge.detail.export', { defaultValue: '导出' })}
                  </Menu.Item>
                  <Menu.Item key='openFolder' onClick={() => void handleOpenFolder()}>
                    {t('knowledge.actions.openFolder', { defaultValue: '打开文件夹' })}
                  </Menu.Item>
                  <Menu.Item key='delete' className='!text-danger-6' onClick={() => setTab('set')}>
                    {t('knowledge.detail.delete', { defaultValue: '删除知识库' })}
                  </Menu.Item>
                </Menu>
              }
              position='br'
            >
              <Button shape='round' icon={<More theme='outline' size='14' />} />
            </Dropdown>
          </div>
        </div>

        {/* ─── Meta info row ─────────────────────────────────────────────────── */}
        {base && (
          <div className='flex flex-wrap gap-14px text-12px text-[var(--color-text-3)]'>
            <span>{t('knowledge.detail.fileCount', { defaultValue: '{{n}} 篇文档', n: base.file_count })}</span>
            <span>{formatSize(base.total_size)}</span>
            {/* mount count placeholder — D3 consumers section will provide real data */}
            <span>{t('knowledge.detail.rootPath', { defaultValue: '{{path}}', path: base.root_path })}</span>
            {relativeTime && (
              <span>{t('knowledge.detail.updatedAt', { defaultValue: '更新于 {{time}}', time: relativeTime })}</span>
            )}
          </div>
        )}

        {/* ─── Tabs ──────────────────────────────────────────────────────────── */}
        <Tabs className='knowledge-detail-tabs' activeTab={activeTab} onChange={(k) => setTab(k)} type='line'>
          {/* Tab: Documents */}
          <Tabs.TabPane key='docs' title={t('knowledge.detail.tabDocs', { defaultValue: '文档' })}>
            {/* ── Document tree + viewer (D2 redesign) ── */}
            <div
              className={classNames(
                'knowledge-doc-workspace flex w-full gap-14px',
                isMobile ? 'flex-col' : 'flex-row',
                isMobile ? 'min-h-720px' : 'h-[clamp(500px,calc(100vh-300px),760px)] min-h-500px'
              )}
            >
              {/* ─── Left: File tree panel ─── */}
              <div
                className={classNames(
                  'knowledge-doc-panel-frame knowledge-doc-sidebar box-border shrink-0 flex flex-col overflow-hidden rd-12px bg-transparent',
                  isMobile ? 'h-420px w-full' : 'h-full w-276px'
                )}
              >
                {/* Compact document toolbar: icon-first, labels are shown in small hover bubbles. */}
                <div className='knowledge-doc-divider-bottom knowledge-doc-toolbar flex h-42px shrink-0 items-center gap-2px bg-transparent px-9px'>
                  {id && base && (
                    <KnowledgeAddContentControl
                      key={id}
                      ref={addContentControlRef}
                      knowledgeBaseId={id}
                      baseRootPath={base.root_path}
                      defaultFolderPath={selectedFolderPath || parentDirOfKnowledgePath(selectedPath)}
                      existingUrlCount={source?.entries.length ?? 0}
                      onAdded={handleContentAdded}
                    />
                  )}
                  <KnowledgeIconButton
                    label={t('knowledge.detail.docs.newFolder', { defaultValue: '新建文件夹' })}
                    icon={<FolderPlus theme='outline' size='15' />}
                    onClick={() => openNewFolderModal()}
                  />
                  <div className='ml-auto flex items-center gap-2px'>
                    <KnowledgeIconButton
                      label={t('knowledge.detail.docs.revealCurrentFile', { defaultValue: '自动显示当前文件' })}
                      icon={<FileFocus theme='outline' size='15' />}
                      loading={treeAction === 'reveal'}
                      onClick={() => void handleRevealCurrentFile()}
                    />
                    <KnowledgeIconButton
                      label={
                        isEntireTreeExpanded && !isTreeSearch
                          ? t('knowledge.detail.docs.collapseAll', { defaultValue: '全部折叠' })
                          : t('knowledge.detail.docs.expandAll', { defaultValue: '全部展开' })
                      }
                      icon={
                        isEntireTreeExpanded && !isTreeSearch ? (
                          <ExpandUp theme='outline' size='15' />
                        ) : (
                          <ExpandDown theme='outline' size='15' />
                        )
                      }
                      loading={treeAction === 'expand'}
                      onClick={handleToggleEntireTree}
                    />
                  </div>
                </div>

                {/* Search box */}
                <div className='knowledge-doc-search mx-9px mt-9px flex shrink-0 items-center gap-7px rounded-7px bg-[var(--color-fill-2)] border border-solid border-[var(--color-border-3)] px-9px py-4px'>
                  <Search theme='outline' size='13' className='text-[var(--color-text-3)] shrink-0' />
                  <input
                    className='min-w-0 border-none bg-transparent outline-none text-[var(--color-text-1)] text-11px w-full placeholder:text-[var(--color-text-3)]'
                    placeholder={t('knowledge.detail.docs.searchPlaceholder', { defaultValue: '搜索文档…' })}
                    value={fileSearch}
                    onChange={(e) => setFileSearch(e.target.value)}
                  />
                </div>

                {/* File tree */}
                <div
                  ref={treeScrollRef}
                  className='knowledge-doc-tree-scroll min-h-0 flex-1 overflow-y-auto px-7px py-8px'
                >
                  <Spin loading={loading} className='w-full'>
                    {displayedTreeData.length === 0 ? (
                      <Empty
                        description={
                          fileSearch.trim()
                            ? t('knowledge.detail.docs.noSearchResults', { defaultValue: '无匹配文件' })
                            : t('knowledge.noFiles')
                        }
                        className='mt-32px'
                      />
                    ) : (
                      <Tree
                        className='knowledge-doc-tree text-13px [&_.arco-tree-node]:w-full [&_.arco-tree-node-title-wrapper]:flex [&_.arco-tree-node-title-wrapper]:w-full [&_.arco-tree-node-title-wrapper]:min-w-0 [&_.arco-tree-node-title-wrapper]:items-center [&_.arco-tree-node-title]:min-w-0 [&_.arco-tree-node-title]:flex-1 [&_.arco-tree-node-title]:!pr-0'
                        size='mini'
                        blockNode
                        showLine
                        icons={(nodeProps) => ({
                          switcherIcon: nodeProps.isLeaf ? null : (
                            <Right
                              theme='outline'
                              size='11'
                              className={classNames(
                                'knowledge-tree-switcher-chevron transition-transform duration-150',
                                nodeProps.expanded && 'rotate-90'
                              )}
                            />
                          ),
                        })}
                        actionOnClick={['select', 'expand']}
                        selectedKeys={selectedTreeKey ? [selectedTreeKey] : []}
                        expandedKeys={visibleTreeExpandedKeys}
                        treeData={displayedTreeData}
                        fieldNames={{
                          children: 'children',
                          title: 'name',
                          key: 'rel_path',
                          isLeaf: 'is_file',
                        }}
                        onSelect={(_keys, extra) => {
                          const dataRef = (extra?.node as { props?: { dataRef?: IKnowledgeTreeEntry } } | undefined)
                            ?.props?.dataRef;
                          if (!dataRef) return;
                          setSelectedTreeKey(dataRef.rel_path);
                          if (dataRef.is_file) {
                            setSelectedPath(dataRef.rel_path);
                            setSelectedFolderPath(parentDirOfKnowledgePath(dataRef.rel_path));
                          } else {
                            setSelectedFolderPath(dataRef.rel_path);
                          }
                        }}
                        onExpand={(keys) => {
                          if (!isTreeSearch) setExpandedTreeKeys(keys.map(String));
                        }}
                        loadMore={(treeNode) => {
                          const dataRef = (treeNode.props as { dataRef?: IKnowledgeTreeEntry }).dataRef;
                          if (!dataRef || dataRef.is_file || isTreeSearch) return Promise.resolve();
                          return handleLoadTreeChildren(dataRef).catch((e: unknown) => {
                            Message.error(String(e));
                          });
                        }}
                        renderTitle={(node) => {
                          const item = node.dataRef as IKnowledgeTreeEntry;
                          return (
                            <div className='knowledge-tree-node-row group flex w-full min-w-0 items-center gap-3px pr-1px'>
                              <span className='knowledge-tree-node-main flex min-w-0 flex-1 items-center'>
                                <span className='knowledge-tree-node-name block min-w-0 truncate leading-17px' title={item.rel_path}>
                                  {node.title}
                                </span>
                              </span>
                              <span className='knowledge-tree-node-action ml-auto w-21px grid shrink-0 place-items-center opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100'>
                                <Dropdown
                                  trigger='click'
                                  droplist={
                                    <Menu
                                      className='knowledge-tree-node-menu'
                                      onClickMenuItem={(key) => handleTreeNodeMenuClick(String(key), item)}
                                    >
                                      {item.is_dir && (
                                        <>
                                          <Menu.Item key='new-file'>
                                            <span className='inline-flex items-center gap-4px'>
                                              <Plus theme='outline' size='11' />
                                              {t('knowledge.detail.docs.newFile', { defaultValue: '新建文档' })}
                                            </span>
                                          </Menu.Item>
                                          <Menu.Item key='new-folder'>
                                            <span className='inline-flex items-center gap-4px'>
                                              <FolderPlus theme='outline' size='11' />
                                              {t('knowledge.detail.docs.newFolder', { defaultValue: '新建文件夹' })}
                                            </span>
                                          </Menu.Item>
                                        </>
                                      )}
                                      <Menu.Item key='rename'>
                                        <span className='inline-flex items-center gap-4px'>
                                          <EditTwo theme='outline' size='11' />
                                          {t('knowledge.actions.rename', { defaultValue: '重命名' })}
                                        </span>
                                      </Menu.Item>
                                      <Menu.Item key='delete' className='!text-danger-6'>
                                        <span className='inline-flex items-center gap-4px'>
                                          <Delete theme='outline' size='11' />
                                          {t('knowledge.actions.delete', { defaultValue: '删除' })}
                                        </span>
                                      </Menu.Item>
                                    </Menu>
                                  }
                                >
                                  <button
                                    type='button'
                                    className='knowledge-tree-node-more grid h-20px w-20px shrink-0 place-items-center rounded-5px border-0 bg-transparent p-0 text-[var(--color-text-3)] cursor-pointer hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)] focus-visible:outline-none focus-visible:bg-[var(--color-fill-2)]'
                                    onMouseDown={(e) => e.stopPropagation()}
                                    onClick={(e) => e.stopPropagation()}
                                    title={t('common.more', { defaultValue: '更多' })}
                                    aria-label={t('common.more', { defaultValue: '更多' })}
                                  >
                                    <More theme='outline' size='13' />
                                  </button>
                                </Dropdown>
                              </span>
                            </div>
                          );
                        }}
                      />
                    )}
                  </Spin>
                </div>

                {/* This footer remains pinned while only the directory tree scrolls. */}
                <div className='knowledge-doc-divider-top knowledge-doc-footer flex h-44px shrink-0 items-center gap-2px bg-transparent px-8px'>
                  <KnowledgeIconButton
                    label={t('knowledge.actions.aiGenerateOverview')}
                    icon={<MagicHat theme='outline' size='13' />}
                    loading={autogenLoading}
                    tooltipPosition='top'
                    onClick={() => void handleAutogen()}
                  />
                  <div className='min-w-0 flex-1'>
                    <KnowledgeModelSelector
                      size='small'
                      choice={modelChoice}
                      onChange={(c) => void setModelChoice(c)}
                      triggerClassName='knowledge-doc-model-trigger'
                    />
                  </div>
                  {source && (
                    <KnowledgeIconButton
                      label={t('knowledge.source.refresh')}
                      icon={<Refresh theme='outline' size='13' />}
                      loading={refreshingSource}
                      tooltipPosition='top'
                      onClick={() => void handleRefreshSource()}
                    />
                  )}
                </div>
              </div>

              {/* ─── Right: Viewer / editor panel ─── */}
              <div className='knowledge-doc-panel-frame box-border min-h-0 min-w-0 flex-1 flex flex-col overflow-hidden rd-12px bg-transparent'>
                {selectedPath == null ? (
                  <div className='flex-1 grid place-items-center'>
                    <Empty description={t('knowledge.selectFile')} />
                  </div>
                ) : (
                  <>
                    {/* Toolbar: breadcrumb + toggle + save */}
                    <div className='knowledge-doc-divider-bottom knowledge-doc-editor-toolbar flex items-center justify-between gap-8px bg-transparent px-16px py-11px'>
                      {/* Breadcrumb */}
                      <div className='text-12px text-[var(--color-text-3)] truncate'>
                        {breadcrumbSegments.map((seg, idx) => (
                          <React.Fragment key={idx}>
                            {idx > 0 && <span className='mx-4px'>/</span>}
                            {idx === breadcrumbSegments.length - 1 ? (
                              <span className='font-500 text-[var(--color-text-2)]'>{seg}</span>
                            ) : (
                              <span>{seg}</span>
                            )}
                          </React.Fragment>
                        ))}
                      </div>
                      {/* Right side controls */}
                      <div className='flex items-center gap-10px shrink-0'>
                        {/* Preview / Edit segmented toggle */}
                        <div className='inline-flex bg-[var(--color-fill-2)] border border-solid border-[var(--color-border-3)] rd-8px p-2px'>
                          <button
                            className={classNames(
                              'bg-transparent text-12px px-12px py-5px rd-6px cursor-pointer font-inherit transition-colors',
                              !editMode
                                ? `${knowledgeDetailSoftActiveClass} font-600`
                                : knowledgeDetailSegmentIdleClass
                            )}
                            onClick={() => setEditMode(false)}
                          >
                            {t('knowledge.detail.docs.preview', { defaultValue: '预览' })}
                          </button>
                          <button
                            className={classNames(
                              'bg-transparent text-12px px-12px py-5px rd-6px cursor-pointer font-inherit transition-colors',
                              editMode
                                ? `${knowledgeDetailSoftActiveClass} font-600`
                                : knowledgeDetailSegmentIdleClass
                            )}
                            onClick={startEdit}
                          >
                            {t('knowledge.detail.docs.edit', { defaultValue: '编辑' })}
                          </button>
                        </div>
                        {/* Save button (visible when editing) */}
                        {editMode && (
                          <Button size='small' type='primary' loading={saving} onClick={() => void handleSave()}>
                            {t('knowledge.actions.save')}
                          </Button>
                        )}
                      </div>
                    </div>
                    {/* Content area */}
                    <div
                      className={classNames(
                        'knowledge-doc-content flex-1 overflow-y-auto',
                        editMode ? 'knowledge-doc-content-edit' : 'p-16px'
                      )}
                    >
                      <Spin loading={fileLoading} className='w-full'>
                        {editMode ? (
                          <Input.TextArea
                            value={draft}
                            onChange={setDraft}
                            autoSize={{ minRows: 18, maxRows: 40 }}
                            className='knowledge-doc-source-editor font-mono text-13px'
                          />
                        ) : (
                          <Markdown compact>{content}</Markdown>
                        )}
                      </Spin>
                    </div>
                  </>
                )}
              </div>
            </div>
          </Tabs.TabPane>

          {/* Tab: Mount & Usage */}
          <Tabs.TabPane key='use' title={t('knowledge.detail.tabUse', { defaultValue: '挂载与使用' })}>
            <div
              className={classNames(
                'knowledge-use-shell grid min-h-470px overflow-hidden rd-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)]',
                isMobile ? 'grid-cols-1' : 'grid-cols-[minmax(0,1fr)_320px]'
              )}
            >
              <div className='flex min-w-0 flex-col gap-14px p-16px'>
                {base ? <KnowledgeConsumersSection baseId={base.knowledge_base_id} /> : null}

                <section className='box-border rd-9px bg-[var(--color-fill-1)] px-12px py-10px'>
                  <div className='text-12px font-700 leading-18px text-[var(--color-text-1)]'>
                    {t('knowledge.detail.use.cliTitle', { defaultValue: '终端 CLI 接入' })}
                  </div>
                  <p className='mb-8px mt-3px text-11px leading-17px text-[var(--color-text-3)]'>
                    {t('knowledge.detail.use.cliDesc', {
                      defaultValue: '给 claude / codex / gemini 一键注入只读的 knowledge_search 工具，让命令行里的 Agent 也能查这个库。请在终端页面使用「接入知识库」按钮完成注册。',
                    })}
                  </p>
                  <Button
                    size='mini'
                    icon={<LinkCloud theme='outline' size='13' />}
                    onClick={() => navigate('/terminal')}
                  >
                    {t('knowledge.detail.use.goTerminal', { defaultValue: '前往终端注册' })}
                  </Button>
                </section>
              </div>

              <aside
                className={classNames(
                  'knowledge-use-rules box-border min-w-0 p-16px',
                  isMobile ? 'knowledge-use-rules-mobile' : 'knowledge-use-rules-desktop'
                )}
              >
                <h3 className='m-0 text-14px font-700 leading-20px text-[var(--color-text-1)]'>
                  {t('knowledge.detail.use.rulesTitle', { defaultValue: '使用规则' })}
                </h3>

                <div className='mt-18px flex flex-col gap-18px'>
                  <div className='knowledge-use-step'>
                    <div className='knowledge-use-step-number'>1</div>
                    <div className='min-w-0 pt-2px'>
                      <b className='block text-12px leading-18px text-[var(--color-text-1)]'>
                        {t('knowledge.detail.use.step1Title', { defaultValue: '挂载到一个会话' })}
                      </b>
                      <p className='mb-0 mt-3px text-11px leading-17px text-[var(--color-text-3)]'>
                        {t('knowledge.detail.use.step1Desc', {
                          defaultValue: '把知识库挂到会话 / 终端 / 数字伙伴上，它就成为该处模型的扩展知识。一个库可被多处复用。',
                        })}
                      </p>
                    </div>
                  </div>

                  <div className='knowledge-use-step'>
                    <div className='knowledge-use-step-number'>2</div>
                    <div className='min-w-0 pt-2px'>
                      <b className='block text-12px leading-18px text-[var(--color-text-1)]'>
                        {t('knowledge.detail.use.step2Title', { defaultValue: '模型自动检索' })}
                      </b>
                      <p className='mb-0 mt-3px text-11px leading-17px text-[var(--color-text-3)]'>
                        {t('knowledge.detail.use.step2Desc', {
                          defaultValue: '模型会在 .nomi/knowledge/ 下按需检索，命中的内容用于回答——原文不塞进上下文，省 token。',
                        })}
                      </p>
                    </div>
                  </div>

                  <div className='knowledge-use-step'>
                    <div className='knowledge-use-step-number'>3</div>
                    <div className='min-w-0 pt-2px'>
                      <b className='block text-12px leading-18px text-[var(--color-text-1)]'>
                        {t('knowledge.detail.use.step3Title', { defaultValue: '（可选）回血沉淀' })}
                      </b>
                      <p className='mb-0 mt-3px text-11px leading-17px text-[var(--color-text-3)]'>
                        {t('knowledge.detail.use.step3Desc', {
                          defaultValue: '开启回血后，会话里新学到的知识会直接写回知识库正文，知识库越用越厚。',
                        })}
                      </p>
                      <div className='mt-9px rd-8px bg-[var(--color-fill-1)] px-9px py-8px text-10px leading-16px text-[var(--color-text-3)]'>
                        <div className='font-600 text-[var(--color-text-2)]'>
                          {t('knowledge.detail.use.writebackTitle', { defaultValue: '回血（让会话把新知识写回本库）' })}
                        </div>
                        <p className='mb-5px mt-2px'>
                          {t('knowledge.detail.use.writebackDesc', {
                            defaultValue: '回血在每个会话的「挂载知识库」控件里按工作区设置——不是全局统一开关。每个挂载可独立选择：',
                          })}
                        </p>
                        <ul className='m-0 pl-14px'>
                          <li>
                            <span className='font-500 text-[var(--color-text-2)]'>
                              {t('knowledge.detail.use.writebackOff', { defaultValue: '关闭' })}
                            </span>
                            {' — '}
                            {t('knowledge.detail.use.writebackOffDesc', { defaultValue: '纯只读，不回写' })}
                          </li>
                          <li>
                            <span className='font-500 text-[var(--color-text-2)]'>
                              {t('knowledge.detail.use.writebackDirect', { defaultValue: '开启回写' })}
                            </span>
                            {' — '}
                            {t('knowledge.detail.use.writebackDirectDesc', {
                              defaultValue: '模型把新知识写进库内正文，更新已有文档时追加、不覆盖；由「回写意识」决定它是等你开口还是自己判断',
                            })}
                          </li>
                        </ul>
                      </div>
                    </div>
                  </div>
                </div>
              </aside>
            </div>
          </Tabs.TabPane>

          {/* Tab: Settings (D5) */}
          <Tabs.TabPane
            key='set'
            title={
              <span className='flex items-center gap-6px'>
                <SettingTwo theme='outline' size='13' />
                {t('knowledge.detail.tabSettings', { defaultValue: '设置' })}
              </span>
            }
          >
            <div>
              {base && (
                <SettingsTab
                  base={base}
                  allTags={allTags}
                  createTag={createTag}
                  onRefresh={refresh}
                />
              )}
            </div>
          </Tabs.TabPane>
        </Tabs>
      </div>

      <Modal
        title={t('knowledge.newFolder', { defaultValue: '新建文件夹' })}
        visible={newFolderVisible}
        onOk={() => void handleCreateFolder()}
        onCancel={() => setNewFolderVisible(false)}
        autoFocus={false}
      >
        <Input
          placeholder={t('knowledge.newFolderPlaceholder', { defaultValue: '输入文件夹名或相对路径，例如 raw 或 raw/tutorials' })}
          value={newFolderPath}
          onChange={setNewFolderPath}
          onPressEnter={() => void handleCreateFolder()}
        />
      </Modal>

      <Modal
        title={t('knowledge.renameTitle', { defaultValue: '重命名' })}
        visible={renameVisible}
        onOk={() => void handleRenameTreeEntry()}
        onCancel={() => {
          setRenameVisible(false);
          setRenameTarget(null);
          setRenameName('');
        }}
        autoFocus={false}
      >
        <Input
          placeholder={t('knowledge.renamePlaceholder', { defaultValue: '输入新的名称' })}
          value={renameName}
          onChange={setRenameName}
          onPressEnter={() => void handleRenameTreeEntry()}
        />
      </Modal>
    </div>
  );
};

export default KnowledgeDetailPage;
