/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Modal, Result } from '@arco-design/web-react';
import { Download, Plus, Upload } from '@icon-park/react';
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeCanvasSummary } from '../domain';
import {
  canvasErrorMessage,
  mergeCanvases,
  pruneCanvasSelection,
} from './canvasList';
import { creativeStudioCanvasesService } from './canvasServiceAdapter';
import type { CreativeStudioCanvasesCopy } from './copy';
import { resolveCreativeStudioCanvasesCopy } from './copy';
import CreativeStudioCanvasCard from './CreativeStudioCanvasCard';
import styles from './CreativeStudioCanvasesPage.module.css';
import type {
  CreativeStudioCanvasesService,
  CreativeStudioCanvasesSnapshot,
} from './types';

type CanvasesAction =
  | 'create'
  | 'import'
  | 'export'
  | 'rename'
  | 'delete'
  | null;

export interface CreativeStudioCanvasesPageProps {
  service?: CreativeStudioCanvasesService;
  onOpenCanvas?: (canvas: CreativeCanvasSummary) => void;
  copy?: Partial<CreativeStudioCanvasesCopy>;
  initialSnapshot?: CreativeStudioCanvasesSnapshot;
  initialSelectedIds?: readonly string[];
  autoLoad?: boolean;
}

const sameSet = (left: ReadonlySet<string>, right: ReadonlySet<string>) =>
  left.size === right.size && [...left].every((id) => right.has(id));

const CreativeStudioCanvasesPage: React.FC<
  CreativeStudioCanvasesPageProps
> = ({
  service = creativeStudioCanvasesService,
  onOpenCanvas,
  copy: copyOverrides,
  initialSnapshot,
  initialSelectedIds = [],
  autoLoad = true,
}) => {
  const { i18n } = useTranslation();
  const language = i18n.resolvedLanguage || i18n.language;
  const copy = useMemo(
    () => resolveCreativeStudioCanvasesCopy(language, copyOverrides),
    [copyOverrides, language]
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const initial = initialSnapshot ?? {
    status: autoLoad ? ('loading' as const) : ('ready' as const),
    canvases: [],
  };
  const [loadState, setLoadState] = useState(initial.status);
  const [canvases, setCanvases] = useState<CreativeCanvasSummary[]>([
    ...initial.canvases,
  ]);
  const [loadError, setLoadError] = useState(initial.error ?? '');
  const [selectedIds, setSelectedIds] = useState<Set<string>>(
    () => new Set(initialSelectedIds)
  );
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState('');
  const [deleteIds, setDeleteIds] = useState<string[]>([]);
  const [busyAction, setBusyAction] = useState<CanvasesAction>(null);

  const loadCanvases = useCallback(
    async (signal?: AbortSignal) => {
      setLoadState('loading');
      setLoadError('');
      try {
        const loaded = await service.listCanvases(signal);
        if (signal?.aborted) return;
        setCanvases([...loaded]);
        setLoadState('ready');
      } catch (error) {
        if (signal?.aborted) return;
        setLoadError(canvasErrorMessage(error));
        setLoadState('error');
      }
    },
    [service]
  );

  useEffect(() => {
    if (!autoLoad) return;
    const controller = new AbortController();
    void loadCanvases(controller.signal);
    return () => controller.abort();
  }, [autoLoad, loadCanvases]);

  useEffect(() => {
    setSelectedIds((current) => {
      const pruned = pruneCanvasSelection(current, canvases);
      return sameSet(current, pruned) ? current : pruned;
    });
    if (
      editingId &&
      !canvases.some((canvas) => canvas.canvasId === editingId)
    ) {
      setEditingId(null);
      setEditingTitle('');
    }
  }, [canvases, editingId]);

  const selectionActive = selectedIds.size > 0;
  const controlsDisabled = loadState !== 'ready' || busyAction !== null;

  const createCanvas = useCallback(async () => {
    if (busyAction) return;
    setBusyAction('create');
    try {
      const created = await service.createCanvas(
        copy.defaultCanvasTitle(canvases.length + 1)
      );
      setCanvases((current) => mergeCanvases(current, [created]));
      onOpenCanvas?.(created);
    } catch {
      Message.error(copy.createFailed);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, canvases.length, copy, onOpenCanvas, service]);

  const importCanvases = useCallback(
    async (file?: File) => {
      if (!file || busyAction || !service.archiveCapabilities.canImport) return;
      setBusyAction('import');
      try {
        const imported = await service.importCanvasArchive(file);
        setCanvases((current) => mergeCanvases(current, imported));
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

  const exportCanvases = useCallback(
    async (canvasIds: readonly string[]) => {
      if (
        canvasIds.length === 0 ||
        busyAction ||
        !service.archiveCapabilities.canExport
      ) {
        return;
      }
      setBusyAction('export');
      try {
        await service.exportCanvases(canvasIds);
        Message.success(copy.exportSuccess(canvasIds.length));
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
      const renamed = await service.renameCanvas(editingId, title);
      setCanvases((current) => mergeCanvases(current, [renamed]));
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
    const canvasIds = [...deleteIds];
    setBusyAction('delete');
    try {
      await service.deleteCanvases(canvasIds);
      const deleted = new Set(canvasIds);
      setCanvases((current) =>
        current.filter((canvas) => !deleted.has(canvas.canvasId))
      );
      setSelectedIds(
        (current) =>
          new Set([...current].filter((canvasId) => !deleted.has(canvasId)))
      );
      setDeleteIds([]);
    } catch {
      Message.error(copy.deleteFailed);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, copy.deleteFailed, deleteIds, service]);

  const toggleSelected = useCallback(
    (canvas: CreativeCanvasSummary, selected: boolean) => {
      setSelectedIds((current) => {
        const next = new Set(current);
        if (selected) next.add(canvas.canvasId);
        else next.delete(canvas.canvasId);
        return next;
      });
    },
    []
  );

  return (
    <section
      className={styles.page}
      data-creative-studio-canvases
      data-canvases-state={loadState}
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
                  icon={
                    <Download theme='outline' size={16} fill='currentColor' />
                  }
                  loading={busyAction === 'export'}
                  disabled={
                    controlsDisabled ||
                    !service.archiveCapabilities.canExport
                  }
                  title={
                    service.archiveCapabilities.canExport
                      ? undefined
                      : copy.archiveUnavailable
                  }
                  onClick={() => void exportCanvases([...selectedIds])}
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
            {canvases.length > 0 && loadState === 'ready' ? (
              <Button
                disabled={controlsDisabled}
                onClick={() =>
                  setDeleteIds(canvases.map((canvas) => canvas.canvasId))
                }
              >
                {copy.deleteAll}
              </Button>
            ) : null}
            <Button
              icon={<Upload theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'import'}
              disabled={
                controlsDisabled || !service.archiveCapabilities.canImport
              }
              title={
                service.archiveCapabilities.canImport
                  ? undefined
                  : copy.archiveUnavailable
              }
              onClick={() => fileInputRef.current?.click()}
            >
              {copy.importCanvases}
            </Button>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'create'}
              disabled={controlsDisabled}
              onClick={() => void createCanvas()}
            >
              {copy.newCanvas}
            </Button>
          </div>
        </header>

        <input
          ref={fileInputRef}
          hidden
          type='file'
          accept='application/zip,.zip'
          disabled={!service.archiveCapabilities.canImport}
          onChange={(event) => void importCanvases(event.target.files?.[0])}
        />

        {loadState === 'loading' ? (
          <div className={styles.statePanel} data-canvases-loading>
            <p>{copy.loading}</p>
          </div>
        ) : loadState === 'error' ? (
          <div className={styles.resultPanel} data-canvases-error>
            <Result
              status='error'
              title={copy.loadError}
              subTitle={loadError}
              extra={
                <Button onClick={() => void loadCanvases()}>{copy.retry}</Button>
              }
            />
          </div>
        ) : canvases.length === 0 ? (
          <div className={styles.emptyPanel} data-canvases-empty='library'>
            <div>
              <h2>{copy.emptyTitle}</h2>
              <p>{copy.emptyDescription}</p>
            </div>
            <Button
              type='primary'
              icon={<Plus theme='outline' size={16} fill='currentColor' />}
              loading={busyAction === 'create'}
              disabled={busyAction !== null}
              onClick={() => void createCanvas()}
            >
              {copy.newCanvas}
            </Button>
          </div>
        ) : (
          <div className={styles.grid} data-canvases-grid>
            {canvases.map((canvas) => (
              <CreativeStudioCanvasCard
                key={canvas.canvasId}
                canvas={canvas}
                copy={copy}
                language={language}
                selected={selectedIds.has(canvas.canvasId)}
                editing={editingId === canvas.canvasId}
                editingTitle={editingTitle}
                disabled={busyAction !== null}
                exportDisabled={!service.archiveCapabilities.canExport}
                archiveUnavailableMessage={copy.archiveUnavailable}
                onOpen={(item) => onOpenCanvas?.(item)}
                onToggleSelected={toggleSelected}
                onStartRename={(item) => {
                  setEditingId(item.canvasId);
                  setEditingTitle(item.title);
                }}
                onEditingTitleChange={setEditingTitle}
                onSaveRename={() => void saveRename()}
                onCancelRename={() => {
                  setEditingId(null);
                  setEditingTitle('');
                }}
                onExport={(item) => void exportCanvases([item.canvasId])}
                onDelete={(item) => setDeleteIds([item.canvasId])}
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
        getPopupContainer={() =>
          document.getElementById('creative-studio-portal-root') ??
          document.body
        }
        onCancel={() => {
          if (busyAction !== 'delete') setDeleteIds([]);
        }}
        onOk={() => void confirmDelete()}
      >
        <p className={styles.deleteDescription}>
          {copy.deleteDialogDescription(deleteIds.length)}
        </p>
      </Modal>
    </section>
  );
};

export default CreativeStudioCanvasesPage;
