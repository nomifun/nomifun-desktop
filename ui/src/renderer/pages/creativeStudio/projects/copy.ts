/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TFunction } from 'i18next';

import {
  resolveCreativeStudioCanvasesCopy,
  type CreativeStudioCanvasesCopy,
} from '../canvases/copy';

/** @deprecated Historical copy contract for source compatibility only. */
export interface CreativeStudioProjectsCopy {
  libraryLabel: string;
  title: string;
  newProject: string;
  defaultProjectTitle: (index: number) => string;
  importProjects: string;
  archiveUnavailable: string;
  deleteAll: string;
  exportSelected: string;
  deleteSelected: string;
  loading: string;
  loadError: string;
  retry: string;
  emptyTitle: string;
  emptyDescription: string;
  projectStats: (nodeCount: number, connectionCount: number) => string;
  updatedAt: (formattedDate: string) => string;
  openProject: string;
  exportProject: string;
  renameProject: string;
  deleteProject: string;
  selectProject: (title: string) => string;
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

export const canvasCopyToLegacyProjectCopy = (
  copy: CreativeStudioCanvasesCopy
): CreativeStudioProjectsCopy => ({
  libraryLabel: copy.libraryLabel,
  title: copy.title,
  newProject: copy.newCanvas,
  defaultProjectTitle: copy.defaultCanvasTitle,
  importProjects: copy.importCanvases,
  archiveUnavailable: copy.archiveUnavailable,
  deleteAll: copy.deleteAll,
  exportSelected: copy.exportSelected,
  deleteSelected: copy.deleteSelected,
  loading: copy.loading,
  loadError: copy.loadError,
  retry: copy.retry,
  emptyTitle: copy.emptyTitle,
  emptyDescription: copy.emptyDescription,
  projectStats: copy.canvasStats,
  updatedAt: copy.updatedAt,
  openProject: copy.openCanvas,
  exportProject: copy.exportCanvas,
  renameProject: copy.renameCanvas,
  deleteProject: copy.deleteCanvas,
  selectProject: copy.selectCanvas,
  saveRename: copy.saveRename,
  cancelRename: copy.cancelRename,
  renamePlaceholder: copy.renamePlaceholder,
  importSuccess: copy.importSuccess,
  importFailed: copy.importFailed,
  createFailed: copy.createFailed,
  exportSuccess: copy.exportSuccess,
  exportFailed: copy.exportFailed,
  renameFailed: copy.renameFailed,
  deleteFailed: copy.deleteFailed,
  deleteDialogTitle: copy.deleteDialogTitle,
  deleteDialogDescription: copy.deleteDialogDescription,
  cancel: copy.cancel,
  confirmDelete: copy.confirmDelete,
});

export function legacyProjectCopyToCanvasCopy(
  copy: CreativeStudioProjectsCopy
): CreativeStudioCanvasesCopy;
export function legacyProjectCopyToCanvasCopy(
  copy: Partial<CreativeStudioProjectsCopy> | undefined
): Partial<CreativeStudioCanvasesCopy> | undefined;
export function legacyProjectCopyToCanvasCopy(
  copy: Partial<CreativeStudioProjectsCopy> | undefined
): Partial<CreativeStudioCanvasesCopy> | undefined {
  if (!copy) return undefined;
  const mapped: Partial<CreativeStudioCanvasesCopy> = {
    libraryLabel: copy.libraryLabel,
    title: copy.title,
    newCanvas: copy.newProject,
    defaultCanvasTitle: copy.defaultProjectTitle,
    importCanvases: copy.importProjects,
    archiveUnavailable: copy.archiveUnavailable,
    deleteAll: copy.deleteAll,
    exportSelected: copy.exportSelected,
    deleteSelected: copy.deleteSelected,
    loading: copy.loading,
    loadError: copy.loadError,
    retry: copy.retry,
    emptyTitle: copy.emptyTitle,
    emptyDescription: copy.emptyDescription,
    canvasStats: copy.projectStats,
    updatedAt: copy.updatedAt,
    openCanvas: copy.openProject,
    exportCanvas: copy.exportProject,
    renameCanvas: copy.renameProject,
    deleteCanvas: copy.deleteProject,
    selectCanvas: copy.selectProject,
    saveRename: copy.saveRename,
    cancelRename: copy.cancelRename,
    renamePlaceholder: copy.renamePlaceholder,
    importSuccess: copy.importSuccess,
    importFailed: copy.importFailed,
    createFailed: copy.createFailed,
    exportSuccess: copy.exportSuccess,
    exportFailed: copy.exportFailed,
    renameFailed: copy.renameFailed,
    deleteFailed: copy.deleteFailed,
    deleteDialogTitle: copy.deleteDialogTitle,
    deleteDialogDescription: copy.deleteDialogDescription,
    cancel: copy.cancel,
    confirmDelete: copy.confirmDelete,
  };
  return Object.fromEntries(
    Object.entries(mapped).filter(([, value]) => value !== undefined)
  ) as Partial<CreativeStudioCanvasesCopy>;
}

/** @deprecated Use resolveCreativeStudioCanvasesCopy. */
export const resolveCreativeStudioProjectsCopy = (
  translatorOrLanguage: TFunction | string | undefined,
  overrides?: Partial<CreativeStudioProjectsCopy>
): CreativeStudioProjectsCopy => {
  const base = resolveCreativeStudioCanvasesCopy(translatorOrLanguage);
  return {
    ...canvasCopyToLegacyProjectCopy(base),
    ...overrides,
  };
};
