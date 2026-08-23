/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import i18n, { type TFunction } from 'i18next';

export interface CreativeStudioCanvasesCopy {
  libraryLabel: string;
  title: string;
  newCanvas: string;
  defaultCanvasTitle: (index: number) => string;
  importCanvases: string;
  archiveUnavailable: string;
  deleteAll: string;
  exportSelected: string;
  deleteSelected: string;
  loading: string;
  loadError: string;
  retry: string;
  emptyTitle: string;
  emptyDescription: string;
  canvasStats: (nodeCount: number, connectionCount: number) => string;
  updatedAt: (formattedDate: string) => string;
  openCanvas: string;
  exportCanvas: string;
  renameCanvas: string;
  deleteCanvas: string;
  selectCanvas: (title: string) => string;
  saveRename: string;
  cancelRename: string;
  renamePlaceholder: string;
  importSuccess: (count: number) => string;
  importFailed: string;
  createFailed: string;
  exportSuccess: (count: number) => string;
  exportFailed: string;
  renameFailed: string;
  deleteFailed: string;
  deleteDialogTitle: string;
  deleteDialogDescription: (count: number) => string;
  cancel: string;
  confirmDelete: string;
}

type CopyTranslator = TFunction | string | undefined;

const resolveTranslator = (translatorOrLanguage: CopyTranslator): TFunction =>
  typeof translatorOrLanguage === 'function'
    ? translatorOrLanguage
    : i18n.getFixedT(translatorOrLanguage || i18n.language);

export const resolveCreativeStudioCanvasesCopy = (
  translatorOrLanguage: CopyTranslator,
  overrides?: Partial<CreativeStudioCanvasesCopy>
): CreativeStudioCanvasesCopy => {
  const t = resolveTranslator(translatorOrLanguage);
  const base: CreativeStudioCanvasesCopy = {
    libraryLabel: t('creativeStudio.canvases.libraryLabel', {
      defaultValue: 'Canvas library',
    }),
    title: t('creativeStudio.canvases.title', {
      defaultValue: 'Infinite canvas',
    }),
    newCanvas: t('creativeStudio.canvases.newCanvas', {
      defaultValue: 'New canvas',
    }),
    defaultCanvasTitle: (index) =>
      t('creativeStudio.canvases.defaultCanvasTitle', {
        defaultValue: 'Infinite canvas {{index}}',
        index,
      }),
    importCanvases: t('creativeStudio.canvases.importCanvases', {
      defaultValue: 'Import canvas',
    }),
    archiveUnavailable: t('creativeStudio.canvases.archiveUnavailable', {
      defaultValue: 'Canvas archive service is not connected yet',
    }),
    deleteAll: t('creativeStudio.canvases.deleteAll', {
      defaultValue: 'Delete all',
    }),
    exportSelected: t('creativeStudio.canvases.exportSelected', {
      defaultValue: 'Export selected',
    }),
    deleteSelected: t('creativeStudio.canvases.deleteSelected', {
      defaultValue: 'Delete selected',
    }),
    loading: t('creativeStudio.canvases.loading', {
      defaultValue: 'Loading canvases...',
    }),
    loadError: t('creativeStudio.canvases.loadError', {
      defaultValue: 'Could not load canvases',
    }),
    retry: t('creativeStudio.canvases.retry', { defaultValue: 'Retry' }),
    emptyTitle: t('creativeStudio.canvases.emptyTitle', {
      defaultValue: 'No canvases yet',
    }),
    emptyDescription: t('creativeStudio.canvases.emptyDescription', {
      defaultValue:
        'Create a canvas to save its nodes, connections, and appearance independently.',
    }),
    canvasStats: (nodeCount, connectionCount) =>
      t('creativeStudio.canvases.canvasStats', {
        defaultValue: '{{nodeCount}} nodes · {{connectionCount}} connections',
        nodeCount,
        connectionCount,
      }),
    updatedAt: (formattedDate) =>
      t('creativeStudio.canvases.updatedAt', {
        defaultValue: 'Updated {{formattedDate}}',
        formattedDate,
      }),
    openCanvas: t('creativeStudio.canvases.openCanvas', {
      defaultValue: 'Open canvas',
    }),
    exportCanvas: t('creativeStudio.canvases.exportCanvas', {
      defaultValue: 'Export',
    }),
    renameCanvas: t('creativeStudio.canvases.renameCanvas', {
      defaultValue: 'Rename',
    }),
    deleteCanvas: t('creativeStudio.canvases.deleteCanvas', {
      defaultValue: 'Delete',
    }),
    selectCanvas: (title) =>
      t('creativeStudio.canvases.selectCanvas', {
        defaultValue: 'Select {{title}}',
        title,
      }),
    saveRename: t('creativeStudio.canvases.saveRename', {
      defaultValue: 'Save name',
    }),
    cancelRename: t('creativeStudio.canvases.cancelRename', {
      defaultValue: 'Cancel rename',
    }),
    renamePlaceholder: t('creativeStudio.canvases.renamePlaceholder', {
      defaultValue: 'Enter a canvas name',
    }),
    importSuccess: (count) =>
      t('creativeStudio.canvases.importSuccess', {
        defaultValue: 'Imported {{count}} canvases',
        count,
      }),
    importFailed: t('creativeStudio.canvases.importFailed', {
      defaultValue: 'Import failed. Choose a valid canvas archive.',
    }),
    createFailed: t('creativeStudio.canvases.createFailed', {
      defaultValue: 'Could not create the canvas. Try again.',
    }),
    exportSuccess: (count) =>
      t('creativeStudio.canvases.exportSuccess', {
        defaultValue: 'Exported {{count}} canvases',
        count,
      }),
    exportFailed: t('creativeStudio.canvases.exportFailed', {
      defaultValue: 'Export failed. Try again.',
    }),
    renameFailed: t('creativeStudio.canvases.renameFailed', {
      defaultValue: 'Rename failed. Try again.',
    }),
    deleteFailed: t('creativeStudio.canvases.deleteFailed', {
      defaultValue: 'Delete failed. Try again.',
    }),
    deleteDialogTitle: t('creativeStudio.canvases.deleteDialogTitle', {
      defaultValue: 'Delete canvases?',
    }),
    deleteDialogDescription: (count) =>
      t('creativeStudio.canvases.deleteDialogDescription', {
        defaultValue:
          '{{count}} canvases and all of their nodes and connections will be removed. This cannot be undone.',
        count,
      }),
    cancel: t('creativeStudio.canvases.cancel', { defaultValue: 'Cancel' }),
    confirmDelete: t('creativeStudio.canvases.confirmDelete', {
      defaultValue: 'Delete',
    }),
  };
  return { ...base, ...overrides };
};
