/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

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

const zhCN: CreativeStudioProjectsCopy = {
  libraryLabel: '画布库',
  title: '无限画布',
  newProject: '新建画布',
  defaultProjectTitle: (index) => `无限画布 ${index}`,
  importProjects: '导入画布',
  archiveUnavailable: '画布归档服务尚未接入',
  deleteAll: '删除全部',
  exportSelected: '导出选中',
  deleteSelected: '删除选中',
  loading: '正在加载画布...',
  loadError: '加载画布失败',
  retry: '重试',
  emptyTitle: '还没有画布',
  emptyDescription: '新建一个画布后，就可以独立保存节点、连线和画布外观。',
  projectStats: (nodeCount, connectionCount) => `${nodeCount} 个节点 · ${connectionCount} 条连线`,
  updatedAt: (formattedDate) => `更新于 ${formattedDate}`,
  openProject: '打开画布',
  exportProject: '导出',
  renameProject: '重命名',
  deleteProject: '删除',
  selectProject: (title) => `选择 ${title}`,
  saveRename: '保存名称',
  cancelRename: '取消重命名',
  renamePlaceholder: '输入画布名称',
  importSuccess: (count) => `已导入 ${count} 个画布`,
  importFailed: '导入失败，请选择有效的画布压缩包。',
  createFailed: '新建画布失败，请重试。',
  exportSuccess: (count) => `已导出 ${count} 个画布`,
  exportFailed: '导出失败，请重试。',
  renameFailed: '重命名失败，请重试。',
  deleteFailed: '删除失败，请重试。',
  deleteDialogTitle: '删除画布？',
  deleteDialogDescription: (count) => `将删除 ${count} 个画布，里面的节点和连线也会一起移除。`,
  cancel: '取消',
  confirmDelete: '删除',
};

const enUS: CreativeStudioProjectsCopy = {
  libraryLabel: 'Canvas library',
  title: 'Infinite canvas',
  newProject: 'New canvas',
  defaultProjectTitle: (index) => `Infinite canvas ${index}`,
  importProjects: 'Import canvas',
  archiveUnavailable: 'Canvas archive service is not connected yet',
  deleteAll: 'Delete all',
  exportSelected: 'Export selected',
  deleteSelected: 'Delete selected',
  loading: 'Loading canvases...',
  loadError: 'Could not load canvases',
  retry: 'Retry',
  emptyTitle: 'No canvases yet',
  emptyDescription: 'Create a canvas to save its nodes, connections, and appearance independently.',
  projectStats: (nodeCount, connectionCount) =>
    `${nodeCount} node${nodeCount === 1 ? '' : 's'} · ${connectionCount} connection${connectionCount === 1 ? '' : 's'}`,
  updatedAt: (formattedDate) => `Updated ${formattedDate}`,
  openProject: 'Open canvas',
  exportProject: 'Export',
  renameProject: 'Rename',
  deleteProject: 'Delete',
  selectProject: (title) => `Select ${title}`,
  saveRename: 'Save name',
  cancelRename: 'Cancel rename',
  renamePlaceholder: 'Enter a canvas name',
  importSuccess: (count) => `Imported ${count} canvas${count === 1 ? '' : 'es'}`,
  importFailed: 'Import failed. Choose a valid canvas archive.',
  createFailed: 'Could not create the canvas. Try again.',
  exportSuccess: (count) => `Exported ${count} canvas${count === 1 ? '' : 'es'}`,
  exportFailed: 'Export failed. Try again.',
  renameFailed: 'Rename failed. Try again.',
  deleteFailed: 'Delete failed. Try again.',
  deleteDialogTitle: 'Delete canvases?',
  deleteDialogDescription: (count) =>
    `${count} canvas${count === 1 ? '' : 'es'} and all of their nodes and connections will be removed. This cannot be undone.`,
  cancel: 'Cancel',
  confirmDelete: 'Delete',
};

export const resolveCreativeStudioProjectsCopy = (
  language: string | undefined,
  overrides?: Partial<CreativeStudioProjectsCopy>
): CreativeStudioProjectsCopy => {
  const base = language?.toLowerCase().startsWith('zh') ? zhCN : enUS;
  return {
    ...base,
    ...overrides,
  };
};
