/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Modal, Result } from '@arco-design/web-react';
import { Download, Plus, Upload } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import CreativeStudioProjectCard from './CreativeStudioProjectCard';
import { resolveCreativeStudioProjectsCopy, type CreativeStudioProjectsCopy } from './copy';
import { mergeProjects, projectErrorMessage, pruneProjectSelection } from './projectList';
import { creativeStudioProjectsService } from './projectServiceAdapter';
import styles from './CreativeStudioProjectsPage.module.css';
import type {
  CreativeStudioProjectSummary,
  CreativeStudioProjectsService,
  CreativeStudioProjectsSnapshot,
} from './types';

type ProjectsAction = 'create' | 'import' | 'export' | 'rename' | 'delete' | null;

export interface CreativeStudioProjectsPageProps {
  service?: CreativeStudioProjectsService;
  onOpenProject?: (project: CreativeStudioProjectSummary) => void;
  copy?: Partial<CreativeStudioProjectsCopy>;
  initialSnapshot?: CreativeStudioProjectsSnapshot;
  initialSelectedIds?: readonly string[];
  autoLoad?: boolean;
}

const sameSet = (left: ReadonlySet<string>, right: ReadonlySet<string>) =>
  left.size === right.size && [...left].every((id) => right.has(id));

const CreativeStudioProjectsPage: React.FC<CreativeStudioProjectsPageProps> = ({
  service = creativeStudioProjectsService,
  onOpenProject,
  copy: copyOverrides,
  initialSnapshot,
  initialSelectedIds = [],
  autoLoad = true,
}) => {
  const { i18n } = useTranslation();
  const language = i18n.resolvedLanguage || i18n.language;
  const copy = useMemo(
    () => resolveCreativeStudioProjectsCopy(language, copyOverrides),
    [copyOverrides, language]
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const initial = initialSnapshot ?? {
    status: autoLoad ? ('loading' as const) : ('ready' as const),
    projects: [],
  };
  const [loadState, setLoadState] = useState(initial.status);
  const [projects, setProjects] = useState<CreativeStudioProjectSummary[]>([...initial.projects]);
  const [loadError, setLoadError] = useState(initial.error ?? '');
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set(initialSelectedIds));
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState('');
  const [deleteIds, setDeleteIds] = useState<string[]>([]);
  const [busyAction, setBusyAction] = useState<ProjectsAction>(null);

  const loadProjects = useCallback(
    async (signal?: AbortSignal) => {
      setLoadState('loading');
      setLoadError('');
      try {
        const loaded = await service.listProjects(signal);
        if (signal?.aborted) return;
        setProjects([...loaded]);
        setLoadState('ready');
      } catch (error) {
        if (signal?.aborted) return;
        setLoadError(projectErrorMessage(error));
        setLoadState('error');
      }
    },
    [service]
  );

  useEffect(() => {
    if (!autoLoad) return;
    const controller = new AbortController();
    void loadProjects(controller.signal);
    return () => controller.abort();
  }, [autoLoad, loadProjects]);

  useEffect(() => {
    setSelectedIds((current) => {
      const pruned = pruneProjectSelection(current, projects);
      return sameSet(current, pruned) ? current : pruned;
    });
    if (editingId && !projects.some((project) => project.id === editingId)) {
      setEditingId(null);
      setEditingTitle('');
    }
  }, [editingId, projects]);

  const selectionActive = selectedIds.size > 0;
  const controlsDisabled = loadState !== 'ready' || busyAction !== null;

  const createProject = useCallback(async () => {
    if (busyAction) return;
    setBusyAction('create');
    try {
      const created = await service.createProject(copy.defaultProjectTitle(projects.length + 1));
      setProjects((current) => mergeProjects(current, [created]));
      onOpenProject?.(created);
    } catch {
      Message.error(copy.createFailed);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, copy, onOpenProject, projects.length, service]);

  const importProjects = useCallback(
    async (file?: File) => {
      if (!file || busyAction || !service.archiveCapabilities.canImport) return;
      setBusyAction('import');
      try {
        const imported = await service.importProjectArchive(file);
        setProjects((current) => mergeProjects(current, imported));
        Message.success(copy.importSuccess(imported.length));
      } catch {
        Message.error(copy.importFailed);
      } finally {
        setBusyAction(null);
        if (fileInputRef.current) fileInputRef.current.value = '';
      }
    },
    [busyAction, copy, service]
  );

  const exportProjects = useCallback(
    async (ids: readonly string[]) => {
      if (ids.length === 0 || busyAction || !service.archiveCapabilities.canExport) return;
      setBusyAction('export');
      try {
        await service.exportProjects(ids);
        Message.success(copy.exportSuccess(ids.length));
      } catch {
        Message.error(copy.exportFailed);
      } finally {
        setBusyAction(null);
      }
    },
    [busyAction, copy, service]
  );

  const saveRename = useCallback(async () => {
    const title = editingTitle.trim();
    if (!editingId || !title || busyAction) return;
    setBusyAction('rename');
    try {
      const renamed = await service.renameProject(editingId, title);
      setProjects((current) => mergeProjects(current, [renamed]));
      setEditingId(null);
      setEditingTitle('');
    } catch {
      Message.error(copy.renameFailed);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, copy.renameFailed, editingId, editingTitle, service]);

  const confirmDelete = useCallback(async () => {
    if (deleteIds.length === 0 || busyAction) return;
    const ids = [...deleteIds];
    setBusyAction('delete');
    try {
      await service.deleteProjects(ids);
      const deleted = new Set(ids);
      setProjects((current) => current.filter((project) => !deleted.has(project.id)));
      setSelectedIds((current) => new Set([...current].filter((id) => !deleted.has(id))));
      setDeleteIds([]);
    } catch {
      Message.error(copy.deleteFailed);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, copy.deleteFailed, deleteIds, service]);

  const toggleSelected = useCallback((project: CreativeStudioProjectSummary, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(project.id);
      else next.delete(project.id);
      return next;
    });
  }, []);

  return (
    <section
      className={styles.page}
      data-creative-studio-projects
      data-projects-state={loadState}
      data-selection-active={selectionActive ? 'true' : 'false'}
      aria-busy={loadState === 'loading'}
    >
      <div className={styles.container}>
        <header className={styles.header}>
          <div className={styles.heading}>
            <h1 className={styles.title}>{copy.title}</h1>
          </div>
          <div className={styles.headerActions}>
            {selectionActive && loadState === 'ready' ? (
              <>
                <Button
                  icon={<Download theme='outline' size={16} fill='currentColor' />}
                  loading={busyAction === 'export'}
                  disabled={controlsDisabled || !service.archiveCapabilities.canExport}
                  title={service.archiveCapabilities.canExport ? undefined : copy.archiveUnavailable}
                  onClick={() => void exportProjects([...selectedIds])}
                >
                  {copy.exportSelected}
                </Button>
                <Button
                  disabled={controlsDisabled}
                  onClick={() => setDeleteIds([...selectedIds])}
                >
                  {copy.deleteSelected}
                </Button>
              </>
            ) : null}
            {projects.length > 0 && loadState === 'ready' ? (
              <Button
                disabled={controlsDisabled}
                onClick={() => setDeleteIds(projects.map((project) => project.id))}
              >
                {copy.deleteAll}
              </Button>
            ) : null}
            <Button
              icon={<Upload theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'import'}
              disabled={controlsDisabled || !service.archiveCapabilities.canImport}
              title={service.archiveCapabilities.canImport ? undefined : copy.archiveUnavailable}
              onClick={() => fileInputRef.current?.click()}
            >
              {copy.importProjects}
            </Button>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'create'}
              disabled={controlsDisabled}
              onClick={() => void createProject()}
            >
              {copy.newProject}
            </Button>
          </div>
        </header>

        <input
          ref={fileInputRef}
          hidden
          type='file'
          accept='application/zip,.zip'
          disabled={!service.archiveCapabilities.canImport}
          onChange={(event) => void importProjects(event.target.files?.[0])}
        />

        {loadState === 'loading' ? (
          <div className={styles.statePanel} data-projects-loading>
            <p>{copy.loading}</p>
          </div>
        ) : loadState === 'error' ? (
          <div className={styles.resultPanel} data-projects-error>
            <Result
              status='error'
              title={copy.loadError}
              subTitle={loadError}
              extra={<Button onClick={() => void loadProjects()}>{copy.retry}</Button>}
            />
          </div>
        ) : projects.length === 0 ? (
          <div className={styles.emptyPanel} data-projects-empty='library'>
            <div>
              <h2>{copy.emptyTitle}</h2>
              <p>{copy.emptyDescription}</p>
            </div>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'create'}
              disabled={busyAction !== null}
              onClick={() => void createProject()}
            >
              {copy.newProject}
            </Button>
          </div>
        ) : (
          <div className={styles.grid} data-projects-grid>
            {projects.map((project) => (
              <CreativeStudioProjectCard
                key={project.id}
                project={project}
                copy={copy}
                language={language}
                selected={selectedIds.has(project.id)}
                editing={editingId === project.id}
                editingTitle={editingTitle}
                disabled={busyAction !== null}
                exportDisabled={!service.archiveCapabilities.canExport}
                archiveUnavailableMessage={copy.archiveUnavailable}
                onOpen={(item) => onOpenProject?.(item)}
                onToggleSelected={toggleSelected}
                onStartRename={(item) => {
                  setEditingId(item.id);
                  setEditingTitle(item.title);
                }}
                onEditingTitleChange={setEditingTitle}
                onSaveRename={() => void saveRename()}
                onCancelRename={() => {
                  setEditingId(null);
                  setEditingTitle('');
                }}
                onExport={(item) => void exportProjects([item.id])}
                onDelete={(item) => setDeleteIds([item.id])}
              />
            ))}
          </div>
        )}
      </div>

      <Modal
        visible={deleteIds.length > 0}
        title={copy.deleteDialogTitle}
        okText={copy.confirmDelete}
        cancelText={copy.cancel}
        okButtonProps={{ status: 'danger' }}
        confirmLoading={busyAction === 'delete'}
        autoFocus={false}
        unmountOnExit
        getPopupContainer={() => document.getElementById('creative-studio-portal-root') ?? document.body}
        onCancel={() => {
          if (busyAction !== 'delete') setDeleteIds([]);
        }}
        onOk={() => void confirmDelete()}
      >
        <p className={styles.deleteDescription}>{copy.deleteDialogDescription(deleteIds.length)}</p>
      </Modal>
    </section>
  );
};

export default CreativeStudioProjectsPage;
