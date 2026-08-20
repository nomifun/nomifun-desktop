/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Message, Modal, Select, Spin } from '@arco-design/web-react';
import {
  Copy,
  Delete,
  EditTwo,
  MagicWand,
  Pic,
  Play,
  Plus,
  Robot,
} from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';

import {
  cloneWorkflowDefinition,
  validateWorkflowDefinition,
  type WorkflowDefinitionV1,
  type WorkflowVariable,
} from '../domain';
import {
  creativeWorkflowRepository,
  type CreativeWorkflowRepository,
} from '../services';
import styles from './CreativeWorkflowWorkspacePage.module.css';
import WorkflowEditorModal from './WorkflowEditorModal';
import WorkflowRunModal, {
  type CreativeWorkflowRunnerPort,
} from './WorkflowRunModal';
import WorkflowRunCenter, {
  type CreativeWorkflowRunCenterPort,
} from './WorkflowRunCenter';
import {
  createBlankWorkflow,
  duplicateWorkflow,
  workflowOutputLabel,
  workflowPromptPreview,
} from './workflowViewModel';

type PageState = 'loading' | 'ready' | 'error';
type WorkflowAction = 'save' | 'copy' | 'delete' | null;

export interface CreativeWorkflowWorkspacePageProps {
  repository?: CreativeWorkflowRepository;
  runner?: CreativeWorkflowRunnerPort;
  runCenter?: CreativeWorkflowRunCenterPort;
  initialWorkflows?: readonly WorkflowDefinitionV1[];
  autoLoad?: boolean;
  onCreateWithAgent?: () => void;
  onOpenModelSettings?: () => void;
  onPickAssets?: (variable: WorkflowVariable) => Promise<string[]>;
  onPickReferenceAssets?: () => Promise<string[]>;
  onUploadReferenceImages?: (files: readonly File[]) => Promise<string[]>;
}

const newestFirst = (workflows: readonly WorkflowDefinitionV1[]) =>
  [...workflows].sort(
    (left, right) =>
      right.metadata.updatedAt - left.metadata.updatedAt ||
      right.metadata.createdAt - left.metadata.createdAt ||
      right.id.localeCompare(left.id)
  );

const upsertWorkflow = (
  workflows: readonly WorkflowDefinitionV1[],
  workflow: WorkflowDefinitionV1
) => newestFirst([workflow, ...workflows.filter((candidate) => candidate.id !== workflow.id)]);

const formatDate = (timestamp: number): string => {
  const date = new Date(timestamp);
  if (!Number.isFinite(timestamp) || Number.isNaN(date.getTime()) || timestamp === 0) return '刚刚';
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(date);
};

const errorMessage = (error: unknown, fallback: string) =>
  error instanceof Error && error.message.trim() ? error.message : fallback;

const WorkflowCard: React.FC<{
  workflow: WorkflowDefinitionV1;
  disabled: boolean;
  onRun: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
}> = ({ workflow, disabled, onRun, onEdit, onCopy, onDelete }) => (
  <article className={styles.card} data-workflow-id={workflow.id}>
    <div className={styles.cardAccent} aria-hidden='true' />
    <div className={styles.cardBody}>
      <div className={styles.cardHeader}>
        <div className={styles.cardIdentity}>
          <h2 className={styles.cardTitle}>{workflow.metadata.name}</h2>
          <div className={styles.chips}>
            <span className={styles.chip}>{workflow.metadata.category || '未分类'}</span>
            <span
              className={styles.chip}
              data-tone={workflow.output.kind === 'multi-image-series' ? 'purple' : undefined}
            >
              {workflowOutputLabel(workflow.output)}
            </span>
            <span className={styles.chip}>{workflow.variables.length} 个变量</span>
            <span
              className={styles.chip}
              data-tone={workflow.metadata.visibility === 'public' ? 'blue' : undefined}
            >
              {workflow.metadata.visibility === 'public' ? '公开' : '个人'}
            </span>
          </div>
        </div>
        <Button
          type='primary'
          size='small'
          disabled={disabled}
          icon={<Play theme='outline' size={14} fill='currentColor' />}
          onClick={onRun}
        >
          运行
        </Button>
      </div>
      <p className={styles.cardDescription}>{workflow.metadata.description || '暂无描述'}</p>
      <div className={styles.promptPreview}>{workflowPromptPreview(workflow)}</div>
      <footer className={styles.cardFooter}>
        <p className={styles.cardDate}>更新于 {formatDate(workflow.metadata.updatedAt)}</p>
        <div className={styles.cardActions}>
          <button
            type='button'
            className={styles.iconButton}
            aria-label={`编辑 ${workflow.metadata.name}`}
            disabled={disabled}
            onClick={onEdit}
          >
            <EditTwo theme='outline' size={14} fill='currentColor' />
          </button>
          <button
            type='button'
            className={styles.iconButton}
            aria-label={`复制 ${workflow.metadata.name}`}
            disabled={disabled}
            onClick={onCopy}
          >
            <Copy theme='outline' size={14} fill='currentColor' />
          </button>
          <button
            type='button'
            className={styles.iconButton}
            data-danger='true'
            aria-label={`删除 ${workflow.metadata.name}`}
            disabled={disabled}
            onClick={onDelete}
          >
            <Delete theme='outline' size={14} fill='currentColor' />
          </button>
        </div>
      </footer>
    </div>
  </article>
);

const CreativeWorkflowWorkspacePage: React.FC<CreativeWorkflowWorkspacePageProps> = ({
  repository = creativeWorkflowRepository,
  runner,
  runCenter,
  initialWorkflows = [],
  autoLoad = true,
  onCreateWithAgent,
  onOpenModelSettings,
  onPickAssets,
  onPickReferenceAssets,
  onUploadReferenceImages,
}) => {
  const [pageState, setPageState] = useState<PageState>(autoLoad ? 'loading' : 'ready');
  const [loadError, setLoadError] = useState('');
  const [workflows, setWorkflows] = useState(() => newestFirst(initialWorkflows));
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [editing, setEditing] = useState<WorkflowDefinitionV1 | null>(null);
  const [editingIsNew, setEditingIsNew] = useState(false);
  const [running, setRunning] = useState<WorkflowDefinitionV1 | null>(null);
  const [deleting, setDeleting] = useState<WorkflowDefinitionV1 | null>(null);
  const [action, setAction] = useState<WorkflowAction>(null);

  const load = useCallback(async () => {
    setPageState('loading');
    setLoadError('');
    try {
      const loaded = await repository.list();
      setWorkflows(newestFirst(loaded));
      setPageState('ready');
    } catch (error) {
      setLoadError(errorMessage(error, '工作流加载失败'));
      setPageState('error');
    }
  }, [repository]);

  useEffect(() => {
    if (!autoLoad) return;
    let active = true;
    setPageState('loading');
    setLoadError('');
    void repository
      .list()
      .then((loaded) => {
        if (!active) return;
        setWorkflows(newestFirst(loaded));
        setPageState('ready');
      })
      .catch((error: unknown) => {
        if (!active) return;
        setLoadError(errorMessage(error, '工作流加载失败'));
        setPageState('error');
      });
    return () => {
      active = false;
    };
  }, [autoLoad, repository]);

  const categories = useMemo(
    () =>
      [...new Set(workflows.map((workflow) => workflow.metadata.category || '未分类'))].sort(
        (left, right) => left.localeCompare(right, 'zh-CN')
      ),
    [workflows]
  );
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return workflows.filter((workflow) => {
      if (category !== 'all' && (workflow.metadata.category || '未分类') !== category) {
        return false;
      }
      if (!needle) return true;
      return [
        workflow.metadata.name,
        workflow.metadata.category,
        workflow.metadata.description,
        ...workflow.metadata.tags,
      ].some((value) => value.toLocaleLowerCase().includes(needle));
    });
  }, [category, query, workflows]);

  const beginCreate = (mode: 'single-image' | 'multi-image-series') => {
    setEditing(createBlankWorkflow(mode));
    setEditingIsNew(true);
  };

  const saveEditing = async () => {
    if (!editing || action) return;
    const validation = validateWorkflowDefinition(editing);
    if (!validation.ok) {
      Message.error(`${validation.error.path}: ${validation.error.message}`);
      return;
    }
    setAction('save');
    try {
      const saved = editingIsNew
        ? await repository.create({ ...editing, revision: 1 })
        : await repository.save(editing.id, editing.revision, {
            ...editing,
            revision: editing.revision + 1,
          });
      setWorkflows((current) => upsertWorkflow(current, saved));
      setEditing(null);
      setEditingIsNew(false);
      Message.success(editingIsNew ? '工作流已创建' : '工作流已保存');
    } catch (error) {
      Message.error(errorMessage(error, '工作流保存失败'));
    } finally {
      setAction(null);
    }
  };

  const copyWorkflow = async (workflow: WorkflowDefinitionV1) => {
    if (action) return;
    setAction('copy');
    try {
      const created = await repository.create(duplicateWorkflow(workflow));
      setWorkflows((current) => upsertWorkflow(current, created));
      Message.success('工作流副本已创建');
    } catch (error) {
      Message.error(errorMessage(error, '工作流复制失败'));
    } finally {
      setAction(null);
    }
  };

  const deleteWorkflow = async () => {
    if (!deleting || action) return;
    setAction('delete');
    try {
      await repository.remove(deleting.id);
      setWorkflows((current) => current.filter((workflow) => workflow.id !== deleting.id));
      setDeleting(null);
      Message.success('工作流已删除');
    } catch (error) {
      Message.error(errorMessage(error, '工作流删除失败'));
    } finally {
      setAction(null);
    }
  };

  const disabled = pageState !== 'ready' || action !== null;

  return (
    <main
      className={styles.page}
      data-creative-workflow-workspace
      data-page-state={pageState}
      aria-busy={pageState === 'loading'}
    >
      <div className={styles.inner}>
        <section className={styles.headerCard}>
          <div className={styles.headerIdentity}>
            <div className={styles.titleRow}>
              <MagicWand theme='outline' size={20} fill='currentColor' />
              <h1>创作工作流</h1>
            </div>
            <p>把固定提示词和参数沉淀成模板，每次只填写变量即可批量复用。</p>
          </div>
          <div className={styles.headerActions}>
            <Select
              className={styles.categorySelect}
              value={category}
              options={[
                { value: 'all', label: '全部分类' },
                ...categories.map((item) => ({ value: item, label: item })),
              ]}
              disabled={pageState !== 'ready'}
              onChange={setCategory}
            />
            <Input.Search
              className={styles.search}
              allowClear
              value={query}
              placeholder='搜索名称、分类、描述'
              disabled={pageState !== 'ready'}
              onChange={setQuery}
            />
            <Button
              icon={<Robot theme='outline' size={15} fill='currentColor' />}
              disabled={disabled || !onCreateWithAgent}
              title={onCreateWithAgent ? undefined : 'AI 创建网关正在接入'}
              onClick={onCreateWithAgent}
            >
              AI 创建
            </Button>
            <Button
              icon={<Pic theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('multi-image-series')}
            >
              新建多图
            </Button>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('single-image')}
            >
              新建工作流
            </Button>
          </div>
        </section>

        {pageState === 'loading' ? (
          <div className={styles.statePanel}>
            <Spin tip='正在加载工作流…' />
          </div>
        ) : pageState === 'error' ? (
          <div className={styles.errorState} role='alert'>
            <h2>工作流加载失败</h2>
            <p>{loadError}</p>
            <Button onClick={() => void load()}>重试</Button>
          </div>
        ) : filtered.length === 0 ? (
          <div className={styles.emptyState}>
            <h2>{workflows.length === 0 ? '暂无工作流' : '没有匹配的工作流'}</h2>
            <p>
              {workflows.length === 0
                ? '创建一个工作流，把常用提示词、变量和模型配置沉淀下来。'
                : '调整分类或搜索条件后重试。'}
            </p>
            {workflows.length === 0 ? (
              <Button
                type='primary'
                icon={<Plus theme='outline' size={15} fill='currentColor' />}
                onClick={() => beginCreate('single-image')}
              >
                新建工作流
              </Button>
            ) : null}
          </div>
        ) : (
          <section className={styles.grid} aria-label='工作流列表'>
            {filtered.map((workflow) => (
              <WorkflowCard
                key={workflow.id}
                workflow={workflow}
                disabled={action !== null}
                onRun={() => setRunning(cloneWorkflowDefinition(workflow))}
                onEdit={() => {
                  setEditing(cloneWorkflowDefinition(workflow));
                  setEditingIsNew(false);
                }}
                onCopy={() => void copyWorkflow(workflow)}
                onDelete={() => setDeleting(workflow)}
              />
            ))}
          </section>
        )}

        {runCenter ? <WorkflowRunCenter port={runCenter} /> : null}
      </div>

      <WorkflowEditorModal
        workflow={editing}
        isNew={editingIsNew}
        saving={action === 'save'}
        onChange={setEditing}
        onCancel={() => {
          if (action !== 'save') {
            setEditing(null);
            setEditingIsNew(false);
          }
        }}
        onSave={() => void saveEditing()}
        onOpenModelSettings={onOpenModelSettings}
      />
      <WorkflowRunModal
        workflow={running}
        runner={runner}
        onClose={() => setRunning(null)}
        onPickAssets={onPickAssets}
        onPickReferenceAssets={onPickReferenceAssets}
        onUploadReferenceImages={onUploadReferenceImages}
      />
      <Modal
        visible={deleting !== null}
        title='删除工作流'
        okText='删除'
        cancelText='取消'
        okButtonProps={{ status: 'danger' }}
        confirmLoading={action === 'delete'}
        autoFocus={false}
        unmountOnExit
        getPopupContainer={() =>
          document.getElementById('creative-studio-portal-root') ?? document.body
        }
        onCancel={() => action !== 'delete' && setDeleting(null)}
        onOk={() => void deleteWorkflow()}
      >
        确定删除“{deleting?.metadata.name}”吗？此操作不可撤销。
      </Modal>
    </main>
  );
};

export default CreativeWorkflowWorkspacePage;
