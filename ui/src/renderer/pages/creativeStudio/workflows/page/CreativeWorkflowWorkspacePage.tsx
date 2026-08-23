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
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import type { CreativeModelCatalogSnapshot } from '../../models';
import type { WorkflowDraftPort } from '../agent';
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
import WorkflowAgentDraftModal from './WorkflowAgentDraftModal';
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
  withPrivateWorkflowVisibility,
  workflowOutputLabel,
  workflowPromptPreview,
} from './workflowViewModel';
import {
  createWorkflowTranslationCopy,
  formatWorkflowValidationError,
  workflowFallbackError,
  type WorkflowTranslationCopy,
} from '../workflowI18n';

type PageState = 'loading' | 'ready' | 'error';
type WorkflowAction = 'save' | 'copy' | 'delete' | null;
const UNCATEGORIZED_CATEGORY = '__uncategorized__';

export interface CreativeWorkflowWorkspacePageProps {
  repository?: CreativeWorkflowRepository;
  runner?: CreativeWorkflowRunnerPort;
  runCenter?: CreativeWorkflowRunCenterPort;
  initialWorkflows?: readonly WorkflowDefinitionV1[];
  autoLoad?: boolean;
  agentDraftPort?: WorkflowDraftPort;
  agentModelCatalog?: CreativeModelCatalogSnapshot;
  onOpenModelSettings?: () => void;
  onPickAssets?: (
    variable: WorkflowVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
  onPickReferenceAssets?: (selectedAssetIds: readonly string[]) => Promise<string[] | null>;
  onUploadReferenceImages?: (
    files: readonly File[],
    selectedAssetIds: readonly string[]
  ) => Promise<string[]>;
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

const formatDate = (timestamp: number, locale: string, justNow: string): string => {
  const date = new Date(timestamp);
  if (!Number.isFinite(timestamp) || Number.isNaN(date.getTime()) || timestamp === 0) return justNow;
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(date);
};

const WorkflowCard: React.FC<{
  workflow: WorkflowDefinitionV1;
  disabled: boolean;
  copy: WorkflowTranslationCopy;
  t: TFunction;
  locale: string;
  onRun: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
}> = ({ workflow, disabled, copy, t, locale, onRun, onEdit, onCopy, onDelete }) => (
  <article className={styles.card} data-workflow-id={workflow.id}>
    <div className={styles.cardAccent} aria-hidden='true' />
    <div className={styles.cardBody}>
      <div className={styles.cardHeader}>
        <div className={styles.cardIdentity}>
          <h2 className={styles.cardTitle}>{workflow.metadata.name}</h2>
          <div className={styles.chips}>
            <span className={styles.chip}>
              {workflow.metadata.category ||
                t('creativeStudio.workflows.workspace.categoryFallback', {
                  defaultValue: 'Uncategorized',
                })}
            </span>
            <span
              className={styles.chip}
              data-tone={workflow.output.kind === 'multi-image-series' ? 'purple' : undefined}
            >
              {workflowOutputLabel(workflow.output, copy)}
            </span>
            <span className={styles.chip}>
              {t('creativeStudio.workflows.workspace.variableCount', {
                count: workflow.variables.length,
                defaultValue: '{{count}} variables',
              })}
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
          {t('creativeStudio.workflows.workspace.run', { defaultValue: 'Run' })}
        </Button>
      </div>
      <p className={styles.cardDescription}>
        {workflow.metadata.description ||
          t('creativeStudio.workflows.workspace.noDescription', {
            defaultValue: 'No description',
          })}
      </p>
      <div className={styles.promptPreview}>{workflowPromptPreview(workflow, copy)}</div>
      <footer className={styles.cardFooter}>
        <p className={styles.cardDate}>
          {t('creativeStudio.workflows.workspace.updatedAt', {
            date: formatDate(
              workflow.metadata.updatedAt,
              locale,
              t('creativeStudio.workflows.workspace.justNow', { defaultValue: 'Just now' })
            ),
            defaultValue: 'Updated {{date}}',
          })}
        </p>
        <div className={styles.cardActions}>
          <button
            type='button'
            className={styles.iconButton}
            aria-label={t('creativeStudio.workflows.workspace.edit', {
              name: workflow.metadata.name,
              defaultValue: 'Edit {{name}}',
            })}
            disabled={disabled}
            onClick={onEdit}
          >
            <EditTwo theme='outline' size={14} fill='currentColor' />
          </button>
          <button
            type='button'
            className={styles.iconButton}
            aria-label={t('creativeStudio.workflows.workspace.duplicate', {
              name: workflow.metadata.name,
              defaultValue: 'Duplicate {{name}}',
            })}
            disabled={disabled}
            onClick={onCopy}
          >
            <Copy theme='outline' size={14} fill='currentColor' />
          </button>
          <button
            type='button'
            className={styles.iconButton}
            data-danger='true'
            aria-label={t('creativeStudio.workflows.workspace.delete', {
              name: workflow.metadata.name,
              defaultValue: 'Delete {{name}}',
            })}
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
  agentDraftPort,
  agentModelCatalog,
  onOpenModelSettings,
  onPickAssets,
  onPickReferenceAssets,
  onUploadReferenceImages,
}) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const copy = useMemo(() => createWorkflowTranslationCopy(t), [t]);
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
  const [agentDraftOpen, setAgentDraftOpen] = useState(false);

  const load = useCallback(async () => {
    setPageState('loading');
    setLoadError('');
    try {
      const loaded = await repository.list();
      setWorkflows(newestFirst(loaded));
      setPageState('ready');
    } catch (error) {
      setLoadError(
        workflowFallbackError(
          error,
          t,
          'creativeStudio.workflows.workspace.loadError',
          'Failed to load templates'
        )
      );
      setPageState('error');
    }
  }, [repository, t]);

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
        setLoadError(
          workflowFallbackError(
            error,
            t,
            'creativeStudio.workflows.workspace.loadError',
            'Failed to load templates'
          )
        );
        setPageState('error');
      });
    return () => {
      active = false;
    };
  }, [autoLoad, repository, t]);

  const uncategorized = t('creativeStudio.workflows.workspace.categoryFallback', {
    defaultValue: 'Uncategorized',
  });
  const categories = useMemo(
    () =>
      [...new Set(
        workflows.map((workflow) => workflow.metadata.category || UNCATEGORIZED_CATEGORY)
      )].sort((left, right) => {
        const leftLabel = left === UNCATEGORIZED_CATEGORY ? uncategorized : left;
        const rightLabel = right === UNCATEGORIZED_CATEGORY ? uncategorized : right;
        return leftLabel.localeCompare(rightLabel, locale);
      }),
    [locale, uncategorized, workflows]
  );
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return workflows.filter((workflow) => {
      if (
        category !== 'all' &&
        (workflow.metadata.category || UNCATEGORIZED_CATEGORY) !== category
      ) {
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
  }, [category, query, uncategorized, workflows]);

  const beginCreate = (mode: 'single-image' | 'multi-image-series') => {
    setEditing(withPrivateWorkflowVisibility(createBlankWorkflow(mode, copy)));
    setEditingIsNew(true);
  };

  const saveEditing = async () => {
    if (!editing || action) return;
    const privateEditing = withPrivateWorkflowVisibility(editing);
    const validation = validateWorkflowDefinition(privateEditing);
    if (!validation.ok) {
      Message.error(formatWorkflowValidationError(validation.error, t));
      return;
    }
    setAction('save');
    try {
      const saved = editingIsNew
        ? await repository.create({ ...privateEditing, revision: 1 })
        : await repository.save(privateEditing.id, privateEditing.revision, {
            ...privateEditing,
            revision: privateEditing.revision + 1,
          });
      setWorkflows((current) => upsertWorkflow(current, saved));
      setEditing(null);
      setEditingIsNew(false);
      Message.success(
        t(
          editingIsNew
            ? 'creativeStudio.workflows.workspace.createSuccess'
            : 'creativeStudio.workflows.workspace.saveSuccess',
          {
            defaultValue: editingIsNew ? 'Template created' : 'Template saved',
          }
        )
      );
    } catch (error) {
      Message.error(
        workflowFallbackError(
          error,
          t,
          'creativeStudio.workflows.workspace.saveError',
          'Failed to save template'
        )
      );
    } finally {
      setAction(null);
    }
  };

  const copyWorkflow = async (workflow: WorkflowDefinitionV1) => {
    if (action) return;
    setAction('copy');
    try {
      const created = await repository.create(
        withPrivateWorkflowVisibility(duplicateWorkflow(workflow, copy))
      );
      setWorkflows((current) => upsertWorkflow(current, created));
      Message.success(
        t('creativeStudio.workflows.workspace.copySuccess', {
          defaultValue: 'Template copy created',
        })
      );
    } catch (error) {
      Message.error(
        workflowFallbackError(
          error,
          t,
          'creativeStudio.workflows.workspace.copyError',
          'Failed to copy template'
        )
      );
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
      Message.success(
        t('creativeStudio.workflows.workspace.deleteSuccess', {
          defaultValue: 'Template deleted',
        })
      );
    } catch (error) {
      Message.error(
        workflowFallbackError(
          error,
          t,
          'creativeStudio.workflows.workspace.deleteError',
          'Failed to delete template'
        )
      );
    } finally {
      setAction(null);
    }
  };

  const disabled = pageState !== 'ready' || action !== null;
  const agentDraftAvailable = Boolean(agentDraftPort && agentModelCatalog);

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
                <h1>
                  {t('creativeStudio.workflows.workspace.title', {
                    defaultValue: 'Template Studio',
                  })}
                </h1>
              </div>
            <p>
              {t('creativeStudio.workflows.workspace.description', {
                defaultValue:
                  'Turn fixed prompts, variables, and model settings into reusable templates. Fill in the variables each time to generate in batches.',
              })}
            </p>
          </div>
          <div className={styles.headerActions}>
            <Select
              className={styles.categorySelect}
              value={category}
              options={[
                {
                  value: 'all',
                  label: t('creativeStudio.workflows.workspace.allCategories', {
                    defaultValue: 'All categories',
                  }),
                },
                ...categories.map((item) => ({
                  value: item,
                  label: item === UNCATEGORIZED_CATEGORY ? uncategorized : item,
                })),
              ]}
              disabled={pageState !== 'ready'}
              onChange={setCategory}
            />
            <Input.Search
              className={styles.search}
              allowClear
              value={query}
              placeholder={t('creativeStudio.workflows.workspace.searchPlaceholder', {
                defaultValue: 'Search names, categories, and descriptions',
              })}
              disabled={pageState !== 'ready'}
              onChange={setQuery}
            />
            <Button
              icon={<Robot theme='outline' size={15} fill='currentColor' />}
              disabled={disabled || !agentDraftAvailable}
              title={
                agentDraftAvailable
                  ? undefined
                  : t('creativeStudio.workflows.workspace.aiUnavailable', {
                      defaultValue: 'AI draft service is unavailable',
                    })
              }
              onClick={() => setAgentDraftOpen(true)}
            >
              {t('creativeStudio.workflows.workspace.aiCreate', {
                defaultValue: 'Create with AI',
              })}
            </Button>
            <Button
              icon={<Pic theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('multi-image-series')}
            >
              {t('creativeStudio.workflows.workspace.newMulti', {
                defaultValue: 'New multi-image template',
              })}
            </Button>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('single-image')}
            >
              {t('creativeStudio.workflows.workspace.newSingle', {
                defaultValue: 'New template',
              })}
            </Button>
          </div>
        </section>

        {pageState === 'loading' ? (
          <div className={styles.statePanel}>
            <Spin
              tip={t('creativeStudio.workflows.workspace.loading', {
                defaultValue: 'Loading templates…',
              })}
            />
          </div>
        ) : pageState === 'error' ? (
          <div className={styles.errorState} role='alert'>
            <h2>
              {t('creativeStudio.workflows.workspace.loadFailed', {
                defaultValue: 'Failed to load templates',
              })}
            </h2>
            <p>{loadError}</p>
            <Button onClick={() => void load()}>
              {t('creativeStudio.workflows.workspace.retry', { defaultValue: 'Retry' })}
            </Button>
          </div>
        ) : filtered.length === 0 ? (
          <div className={styles.emptyState}>
            <h2>
              {workflows.length === 0
                ? t('creativeStudio.workflows.workspace.noTemplates', {
                    defaultValue: 'No templates yet',
                  })
                : t('creativeStudio.workflows.workspace.noMatches', {
                    defaultValue: 'No matching templates',
                  })}
            </h2>
            <p>
              {workflows.length === 0
                ? t('creativeStudio.workflows.workspace.emptyDescription', {
                    defaultValue:
                      'Create a template to save frequently used prompts, variables, and model settings.',
                  })
                : t('creativeStudio.workflows.workspace.noMatchesDescription', {
                    defaultValue: 'Adjust the category or search filters and try again.',
                  })}
            </p>
            {workflows.length === 0 ? (
              <Button
                type='primary'
                icon={<Plus theme='outline' size={15} fill='currentColor' />}
                onClick={() => beginCreate('single-image')}
              >
                {t('creativeStudio.workflows.workspace.newSingle', {
                  defaultValue: 'New template',
                })}
              </Button>
            ) : null}
          </div>
        ) : (
          <section
            className={styles.grid}
            aria-label={t('creativeStudio.workflows.workspace.listLabel', {
              defaultValue: 'Template list',
            })}
          >
            {filtered.map((workflow) => (
              <WorkflowCard
                key={workflow.id}
                workflow={workflow}
                disabled={action !== null}
                copy={copy}
                t={t}
                locale={locale}
                onRun={() => setRunning(cloneWorkflowDefinition(workflow))}
                onEdit={() => {
                  setEditing(
                    withPrivateWorkflowVisibility(cloneWorkflowDefinition(workflow))
                  );
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

      {agentDraftPort && agentModelCatalog ? (
        <WorkflowAgentDraftModal
          visible={agentDraftOpen}
          catalog={agentModelCatalog}
          port={agentDraftPort}
          onApply={(workflow) => {
            setEditing(withPrivateWorkflowVisibility(workflow));
            setEditingIsNew(true);
            setAgentDraftOpen(false);
          }}
          onClose={() => setAgentDraftOpen(false)}
          onOpenModelSettings={onOpenModelSettings}
        />
      ) : null}
      <WorkflowEditorModal
        workflow={editing}
        isNew={editingIsNew}
        saving={action === 'save'}
        onChange={(workflow) => setEditing(withPrivateWorkflowVisibility(workflow))}
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
        title={t('creativeStudio.workflows.workspace.deleteTitle', {
          defaultValue: 'Delete template',
        })}
        className={styles.confirmModal}
        okText={t('creativeStudio.workflows.workspace.deleteAction', {
          defaultValue: 'Delete',
        })}
        cancelText={t('creativeStudio.workflows.workspace.cancel', {
          defaultValue: 'Cancel',
        })}
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
        {t('creativeStudio.workflows.workspace.deleteConfirm', {
          name: deleting?.metadata.name ?? '',
          defaultValue: 'Delete “{{name}}”? This action cannot be undone.',
        })}
      </Modal>
    </main>
  );
};

export default CreativeWorkflowWorkspacePage;
