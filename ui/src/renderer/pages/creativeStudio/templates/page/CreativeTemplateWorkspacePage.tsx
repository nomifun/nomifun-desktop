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
import type { TemplateDraftPort } from '../agent';
import {
  cloneTemplateDefinition,
  validateTemplateDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateVariable,
} from '../domain';
import {
  creativeTemplateRepository,
  type CreativeTemplateRepository,
} from '../services';
import styles from './CreativeTemplateWorkspacePage.module.css';
import TemplateAgentDraftModal from './TemplateAgentDraftModal';
import TemplateEditorModal from './TemplateEditorModal';
import TemplateRunModal, {
  type CreativeTemplateRunnerPort,
} from './TemplateRunModal';
import TemplateRunCenter, {
  type CreativeTemplateRunCenterPort,
} from './TemplateRunCenter';
import {
  createBlankTemplate,
  duplicateTemplate,
  withPrivateTemplateVisibility,
  templateOutputLabel,
  templatePromptPreview,
} from './templateViewModel';
import {
  createTemplateTranslationCopy,
  formatTemplateValidationError,
  templateFallbackError,
  type CreativeTemplateTranslationCopy,
} from '../templateI18n';

type PageState = 'loading' | 'ready' | 'error';
type TemplateAction = 'save' | 'copy' | 'delete' | null;
const UNCATEGORIZED_CATEGORY = '__uncategorized__';

export interface CreativeTemplateWorkspacePageProps {
  repository?: CreativeTemplateRepository;
  runner?: CreativeTemplateRunnerPort;
  runCenter?: CreativeTemplateRunCenterPort;
  initialTemplates?: readonly CreativeTemplateDefinitionV1[];
  autoLoad?: boolean;
  agentDraftPort?: TemplateDraftPort;
  agentModelCatalog?: CreativeModelCatalogSnapshot;
  onOpenModelSettings?: () => void;
  onPickAssets?: (
    variable: CreativeTemplateVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
  onPickReferenceAssets?: (selectedAssetIds: readonly string[]) => Promise<string[] | null>;
  onUploadReferenceImages?: (
    files: readonly File[],
    selectedAssetIds: readonly string[]
  ) => Promise<string[]>;
}

const newestFirst = (templates: readonly CreativeTemplateDefinitionV1[]) =>
  [...templates].sort(
    (left, right) =>
      right.metadata.updatedAt - left.metadata.updatedAt ||
      right.metadata.createdAt - left.metadata.createdAt ||
      right.id.localeCompare(left.id)
  );

const upsertTemplate = (
  templates: readonly CreativeTemplateDefinitionV1[],
  template: CreativeTemplateDefinitionV1
) => newestFirst([template, ...templates.filter((candidate) => candidate.id !== template.id)]);

const formatDate = (timestamp: number, locale: string, justNow: string): string => {
  const date = new Date(timestamp);
  if (!Number.isFinite(timestamp) || Number.isNaN(date.getTime()) || timestamp === 0) return justNow;
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(date);
};

const TemplateCard: React.FC<{
  template: CreativeTemplateDefinitionV1;
  disabled: boolean;
  copy: CreativeTemplateTranslationCopy;
  t: TFunction;
  locale: string;
  onRun: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
}> = ({ template, disabled, copy, t, locale, onRun, onEdit, onCopy, onDelete }) => (
  <article className={styles.card} data-template-id={template.id}>
    <div className={styles.cardAccent} aria-hidden='true' />
    <div className={styles.cardBody}>
      <div className={styles.cardHeader}>
        <div className={styles.cardIdentity}>
          <h2 className={styles.cardTitle}>{template.metadata.name}</h2>
          <div className={styles.chips}>
            <span className={styles.chip}>
              {template.metadata.category ||
                t('creativeStudio.templates.workspace.categoryFallback', {
                  defaultValue: 'Uncategorized',
                })}
            </span>
            <span
              className={styles.chip}
              data-tone={template.output.kind === 'multi-image-series' ? 'purple' : undefined}
            >
              {templateOutputLabel(template.output, copy)}
            </span>
            <span className={styles.chip}>
              {t('creativeStudio.templates.workspace.variableCount', {
                count: template.variables.length,
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
          {t('creativeStudio.templates.workspace.run', { defaultValue: 'Run' })}
        </Button>
      </div>
      <p className={styles.cardDescription}>
        {template.metadata.description ||
          t('creativeStudio.templates.workspace.noDescription', {
            defaultValue: 'No description',
          })}
      </p>
      <div className={styles.promptPreview}>{templatePromptPreview(template, copy)}</div>
      <footer className={styles.cardFooter}>
        <p className={styles.cardDate}>
          {t('creativeStudio.templates.workspace.updatedAt', {
            date: formatDate(
              template.metadata.updatedAt,
              locale,
              t('creativeStudio.templates.workspace.justNow', { defaultValue: 'Just now' })
            ),
            defaultValue: 'Updated {{date}}',
          })}
        </p>
        <div className={styles.cardActions}>
          <button
            type='button'
            className={styles.iconButton}
            aria-label={t('creativeStudio.templates.workspace.edit', {
              name: template.metadata.name,
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
            aria-label={t('creativeStudio.templates.workspace.duplicate', {
              name: template.metadata.name,
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
            aria-label={t('creativeStudio.templates.workspace.delete', {
              name: template.metadata.name,
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

const CreativeTemplateWorkspacePage: React.FC<CreativeTemplateWorkspacePageProps> = ({
  repository = creativeTemplateRepository,
  runner,
  runCenter,
  initialTemplates = [],
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
  const copy = useMemo(() => createTemplateTranslationCopy(t), [t]);
  const [pageState, setPageState] = useState<PageState>(autoLoad ? 'loading' : 'ready');
  const [loadError, setLoadError] = useState('');
  const [templates, setTemplates] = useState(() => newestFirst(initialTemplates));
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [editing, setEditing] = useState<CreativeTemplateDefinitionV1 | null>(null);
  const [editingIsNew, setEditingIsNew] = useState(false);
  const [running, setRunning] = useState<CreativeTemplateDefinitionV1 | null>(null);
  const [deleting, setDeleting] = useState<CreativeTemplateDefinitionV1 | null>(null);
  const [action, setAction] = useState<TemplateAction>(null);
  const [agentDraftOpen, setAgentDraftOpen] = useState(false);

  const load = useCallback(async () => {
    setPageState('loading');
    setLoadError('');
    try {
      const loaded = await repository.list();
      setTemplates(newestFirst(loaded));
      setPageState('ready');
    } catch (error) {
      setLoadError(
        templateFallbackError(
          error,
          t,
          'creativeStudio.templates.workspace.loadError',
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
        setTemplates(newestFirst(loaded));
        setPageState('ready');
      })
      .catch((error: unknown) => {
        if (!active) return;
        setLoadError(
          templateFallbackError(
            error,
            t,
            'creativeStudio.templates.workspace.loadError',
            'Failed to load templates'
          )
        );
        setPageState('error');
      });
    return () => {
      active = false;
    };
  }, [autoLoad, repository, t]);

  const uncategorized = t('creativeStudio.templates.workspace.categoryFallback', {
    defaultValue: 'Uncategorized',
  });
  const categories = useMemo(
    () =>
      [...new Set(
        templates.map((template) => template.metadata.category || UNCATEGORIZED_CATEGORY)
      )].sort((left, right) => {
        const leftLabel = left === UNCATEGORIZED_CATEGORY ? uncategorized : left;
        const rightLabel = right === UNCATEGORIZED_CATEGORY ? uncategorized : right;
        return leftLabel.localeCompare(rightLabel, locale);
      }),
    [locale, uncategorized, templates]
  );
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return templates.filter((template) => {
      if (
        category !== 'all' &&
        (template.metadata.category || UNCATEGORIZED_CATEGORY) !== category
      ) {
        return false;
      }
      if (!needle) return true;
      return [
        template.metadata.name,
        template.metadata.category,
        template.metadata.description,
        ...template.metadata.tags,
      ].some((value) => value.toLocaleLowerCase().includes(needle));
    });
  }, [category, query, uncategorized, templates]);

  const beginCreate = (mode: 'single-image' | 'multi-image-series') => {
    setEditing(withPrivateTemplateVisibility(createBlankTemplate(mode, copy)));
    setEditingIsNew(true);
  };

  const saveEditing = async () => {
    if (!editing || action) return;
    const privateEditing = withPrivateTemplateVisibility(editing);
    const validation = validateTemplateDefinition(privateEditing);
    if (!validation.ok) {
      Message.error(formatTemplateValidationError(validation.error, t));
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
      setTemplates((current) => upsertTemplate(current, saved));
      setEditing(null);
      setEditingIsNew(false);
      Message.success(
        t(
          editingIsNew
            ? 'creativeStudio.templates.workspace.createSuccess'
            : 'creativeStudio.templates.workspace.saveSuccess',
          {
            defaultValue: editingIsNew ? 'Template created' : 'Template saved',
          }
        )
      );
    } catch (error) {
      Message.error(
        templateFallbackError(
          error,
          t,
          'creativeStudio.templates.workspace.saveError',
          'Failed to save template'
        )
      );
    } finally {
      setAction(null);
    }
  };

  const copyTemplate = async (template: CreativeTemplateDefinitionV1) => {
    if (action) return;
    setAction('copy');
    try {
      const created = await repository.create(
        withPrivateTemplateVisibility(duplicateTemplate(template, copy))
      );
      setTemplates((current) => upsertTemplate(current, created));
      Message.success(
        t('creativeStudio.templates.workspace.copySuccess', {
          defaultValue: 'Template copy created',
        })
      );
    } catch (error) {
      Message.error(
        templateFallbackError(
          error,
          t,
          'creativeStudio.templates.workspace.copyError',
          'Failed to copy template'
        )
      );
    } finally {
      setAction(null);
    }
  };

  const deleteTemplate = async () => {
    if (!deleting || action) return;
    setAction('delete');
    try {
      await repository.remove(deleting.id);
      setTemplates((current) => current.filter((template) => template.id !== deleting.id));
      setDeleting(null);
      Message.success(
        t('creativeStudio.templates.workspace.deleteSuccess', {
          defaultValue: 'Template deleted',
        })
      );
    } catch (error) {
      Message.error(
        templateFallbackError(
          error,
          t,
          'creativeStudio.templates.workspace.deleteError',
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
      data-creative-template-workspace
      data-page-state={pageState}
      aria-busy={pageState === 'loading'}
    >
      <div className={styles.inner}>
        <section className={styles.headerCard}>
            <div className={styles.headerIdentity}>
              <div className={styles.titleRow}>
                <MagicWand theme='outline' size={20} fill='currentColor' />
                <h1>
                  {t('creativeStudio.templates.workspace.title', {
                    defaultValue: 'Template Studio',
                  })}
                </h1>
              </div>
            <p>
              {t('creativeStudio.templates.workspace.description', {
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
                  label: t('creativeStudio.templates.workspace.allCategories', {
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
              placeholder={t('creativeStudio.templates.workspace.searchPlaceholder', {
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
                  : t('creativeStudio.templates.workspace.aiUnavailable', {
                      defaultValue: 'AI draft service is unavailable',
                    })
              }
              onClick={() => setAgentDraftOpen(true)}
            >
              {t('creativeStudio.templates.workspace.aiCreate', {
                defaultValue: 'Create with AI',
              })}
            </Button>
            <Button
              icon={<Pic theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('multi-image-series')}
            >
              {t('creativeStudio.templates.workspace.newMulti', {
                defaultValue: 'New multi-image template',
              })}
            </Button>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={15} fill='currentColor' />}
              disabled={disabled}
              onClick={() => beginCreate('single-image')}
            >
              {t('creativeStudio.templates.workspace.newSingle', {
                defaultValue: 'New template',
              })}
            </Button>
          </div>
        </section>

        {pageState === 'loading' ? (
          <div className={styles.statePanel}>
            <Spin
              tip={t('creativeStudio.templates.workspace.loading', {
                defaultValue: 'Loading templates…',
              })}
            />
          </div>
        ) : pageState === 'error' ? (
          <div className={styles.errorState} role='alert'>
            <h2>
              {t('creativeStudio.templates.workspace.loadFailed', {
                defaultValue: 'Failed to load templates',
              })}
            </h2>
            <p>{loadError}</p>
            <Button onClick={() => void load()}>
              {t('creativeStudio.templates.workspace.retry', { defaultValue: 'Retry' })}
            </Button>
          </div>
        ) : filtered.length === 0 ? (
          <div className={styles.emptyState}>
            <h2>
              {templates.length === 0
                ? t('creativeStudio.templates.workspace.noTemplates', {
                    defaultValue: 'No templates yet',
                  })
                : t('creativeStudio.templates.workspace.noMatches', {
                    defaultValue: 'No matching templates',
                  })}
            </h2>
            <p>
              {templates.length === 0
                ? t('creativeStudio.templates.workspace.emptyDescription', {
                    defaultValue:
                      'Create a template to save frequently used prompts, variables, and model settings.',
                  })
                : t('creativeStudio.templates.workspace.noMatchesDescription', {
                    defaultValue: 'Adjust the category or search filters and try again.',
                  })}
            </p>
            {templates.length === 0 ? (
              <Button
                type='primary'
                icon={<Plus theme='outline' size={15} fill='currentColor' />}
                onClick={() => beginCreate('single-image')}
              >
                {t('creativeStudio.templates.workspace.newSingle', {
                  defaultValue: 'New template',
                })}
              </Button>
            ) : null}
          </div>
        ) : (
          <section
            className={styles.grid}
            aria-label={t('creativeStudio.templates.workspace.listLabel', {
              defaultValue: 'Template list',
            })}
          >
            {filtered.map((template) => (
              <TemplateCard
                key={template.id}
                template={template}
                disabled={action !== null}
                copy={copy}
                t={t}
                locale={locale}
                onRun={() => setRunning(cloneTemplateDefinition(template))}
                onEdit={() => {
                  setEditing(
                    withPrivateTemplateVisibility(cloneTemplateDefinition(template))
                  );
                  setEditingIsNew(false);
                }}
                onCopy={() => void copyTemplate(template)}
                onDelete={() => setDeleting(template)}
              />
            ))}
          </section>
        )}

        {runCenter ? <TemplateRunCenter port={runCenter} /> : null}
      </div>

      {agentDraftPort && agentModelCatalog ? (
        <TemplateAgentDraftModal
          visible={agentDraftOpen}
          catalog={agentModelCatalog}
          port={agentDraftPort}
          onApply={(template) => {
            setEditing(withPrivateTemplateVisibility(template));
            setEditingIsNew(true);
            setAgentDraftOpen(false);
          }}
          onClose={() => setAgentDraftOpen(false)}
          onOpenModelSettings={onOpenModelSettings}
        />
      ) : null}
      <TemplateEditorModal
        template={editing}
        isNew={editingIsNew}
        saving={action === 'save'}
        onChange={(template) => setEditing(withPrivateTemplateVisibility(template))}
        onCancel={() => {
          if (action !== 'save') {
            setEditing(null);
            setEditingIsNew(false);
          }
        }}
        onSave={() => void saveEditing()}
        onOpenModelSettings={onOpenModelSettings}
      />
      <TemplateRunModal
        template={running}
        runner={runner}
        onClose={() => setRunning(null)}
        onPickAssets={onPickAssets}
        onPickReferenceAssets={onPickReferenceAssets}
        onUploadReferenceImages={onUploadReferenceImages}
      />
      <Modal
        visible={deleting !== null}
        title={t('creativeStudio.templates.workspace.deleteTitle', {
          defaultValue: 'Delete template',
        })}
        className={styles.confirmModal}
        okText={t('creativeStudio.templates.workspace.deleteAction', {
          defaultValue: 'Delete',
        })}
        cancelText={t('creativeStudio.templates.workspace.cancel', {
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
        onOk={() => void deleteTemplate()}
      >
        {t('creativeStudio.templates.workspace.deleteConfirm', {
          name: deleting?.metadata.name ?? '',
          defaultValue: 'Delete “{{name}}”? This action cannot be undone.',
        })}
      </Modal>
    </main>
  );
};

export default CreativeTemplateWorkspacePage;
