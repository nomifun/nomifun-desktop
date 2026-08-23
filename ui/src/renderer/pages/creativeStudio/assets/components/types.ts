/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TFunction } from 'i18next';

import type { CreativeAsset, CreativeAssetKind } from '../types';
import type { UseCreativeAssetsResult } from '../useCreativeAssets';

export type CreativeAssetKindFilter = 'all' | CreativeAssetKind;
export type CreativeAssetScope = 'library' | 'canvas';
export type CreativeAssetViewMode = 'grid' | 'list';
export type CreativeAssetLibraryAppearance = 'default' | 'source-page';

export interface CreativeAssetPagination {
  page: number;
  pageSize: number;
  total: number;
  loading?: boolean;
  onPageChange: (page: number) => void;
}

export type CreativeAssetLibraryState = Pick<
  UseCreativeAssetsResult,
  | 'assets'
  | 'total'
  | 'loading'
  | 'loadingMore'
  | 'mutating'
  | 'error'
  | 'mutationError'
  | 'hasMore'
  | 'reload'
  | 'loadMore'
>;

export type CreativeAssetUploadStatus = 'uploading' | 'completed' | 'error';

export interface CreativeAssetUploadItem {
  id: string;
  fileName: string;
  percent: number;
  status: CreativeAssetUploadStatus;
  error?: string;
}

export interface CreativeTextAssetFormValue {
  title: string;
  textContent: string;
  collection: string;
  tags: string[];
  inLibrary: boolean;
}

export interface CreativeAssetLibraryLabels {
  title: string;
  description: string;
  searchPlaceholder: string;
  clearSearch: string;
  kindFilter: string;
  scopeFilter: string;
  viewFilter: string;
  all: string;
  image: string;
  video: string;
  audio: string;
  text: string;
  libraryScope: string;
  canvasScope: string;
  gridView: string;
  listView: string;
  upload: string;
  createText: string;
  renameCollection: string;
  search: string;
  pagination: string;
  previousPage: string;
  nextPage: string;
  pageSize: (size: number) => string;
  dropFiles: string;
  loading: string;
  loadingMore: string;
  loadMore: string;
  emptyTitle: string;
  emptyDescription: string;
  canvasEmptyTitle: string;
  canvasEmptyDescription: string;
  filteredEmptyTitle: string;
  filteredEmptyDescription: string;
  retry: string;
  select: string;
  selectAll: string;
  clearSelection: string;
  selectedCount: (count: number) => string;
  resultCount: (visible: number, total: number) => string;
  addToLibrary: string;
  removeFromLibrary: string;
  insertIntoCanvas: string;
  downloadSelected: string;
  deleteSelected: string;
  open: string;
  edit: string;
  download: string;
  remove: string;
  noCollection: string;
  noTags: string;
  mediaUnavailable: string;
  uploadQueue: string;
  cancelUpload: string;
  retryUpload: string;
  dismissUpload: string;
  uploadComplete: string;
}

export interface CreateTextAssetLabels {
  title: string;
  description: string;
  titleLabel: string;
  titlePlaceholder: string;
  contentLabel: string;
  contentPlaceholder: string;
  collectionLabel: string;
  collectionPlaceholder: string;
  tagsLabel: string;
  tagsPlaceholder: string;
  saveToLibrary: string;
  cancel: string;
  submit: string;
  submitting: string;
  requiredHint: string;
}

export type CreativeAssetAction = (asset: CreativeAsset) => void;
export type CreativeAssetBatchAction = (assets: readonly CreativeAsset[]) => void;

export const createCreativeAssetLibraryLabels = (t: TFunction): CreativeAssetLibraryLabels => ({
  title: t('creativeStudio.assets.library.title', { defaultValue: '素材库' }),
  description: t('creativeStudio.assets.library.description', { defaultValue: '集中管理画布使用的图片、视频、音频与文本素材。' }),
  searchPlaceholder: t('creativeStudio.assets.library.searchPlaceholder', { defaultValue: '搜索标题、合集或标签' }),
  clearSearch: t('creativeStudio.assets.library.clearSearch', { defaultValue: '清除搜索' }),
  kindFilter: t('creativeStudio.assets.library.kindFilter', { defaultValue: '素材类型' }),
  scopeFilter: t('creativeStudio.assets.library.scopeFilter', { defaultValue: '素材范围' }),
  viewFilter: t('creativeStudio.assets.library.viewFilter', { defaultValue: '显示方式' }),
  all: t('creativeStudio.assets.kind.all', { defaultValue: '全部' }),
  image: t('creativeStudio.assets.kind.image', { defaultValue: '图片' }),
  video: t('creativeStudio.assets.kind.video', { defaultValue: '视频' }),
  audio: t('creativeStudio.assets.kind.audio', { defaultValue: '音频' }),
  text: t('creativeStudio.assets.kind.text', { defaultValue: '文本' }),
  libraryScope: t('creativeStudio.assets.scope.library', { defaultValue: '素材库' }),
  canvasScope: t('creativeStudio.assets.scope.canvas', { defaultValue: '画布素材' }),
  gridView: t('creativeStudio.assets.view.grid', { defaultValue: '网格视图' }),
  listView: t('creativeStudio.assets.view.list', { defaultValue: '列表视图' }),
  upload: t('creativeStudio.assets.library.upload', { defaultValue: '上传素材' }),
  createText: t('creativeStudio.assets.library.createText', { defaultValue: '新建文本' }),
  renameCollection: t('creativeStudio.assets.library.renameCollection', { defaultValue: '重命名合集' }),
  search: t('creativeStudio.assets.library.search', { defaultValue: '搜索' }),
  pagination: t('creativeStudio.assets.library.pagination', { defaultValue: '素材分页' }),
  previousPage: t('creativeStudio.assets.library.previousPage', { defaultValue: '上一页' }),
  nextPage: t('creativeStudio.assets.library.nextPage', { defaultValue: '下一页' }),
  pageSize: (size) => t('creativeStudio.assets.library.pageSize', { defaultValue: '{{size}} 条/页', size }),
  dropFiles: t('creativeStudio.assets.library.dropFiles', { defaultValue: '释放文件以上传' }),
  loading: t('creativeStudio.assets.library.loading', { defaultValue: '正在加载素材' }),
  loadingMore: t('creativeStudio.assets.library.loadingMore', { defaultValue: '正在加载更多' }),
  loadMore: t('creativeStudio.assets.library.loadMore', { defaultValue: '加载更多' }),
  emptyTitle: t('creativeStudio.assets.library.emptyTitle', { defaultValue: '还没有素材' }),
  emptyDescription: t('creativeStudio.assets.library.emptyDescription', { defaultValue: '上传媒体或创建文本素材，开始构建你的创作资源库。' }),
  canvasEmptyTitle: t('creativeStudio.assets.library.canvasEmptyTitle', { defaultValue: '当前画布还没有素材' }),
  canvasEmptyDescription: t('creativeStudio.assets.library.canvasEmptyDescription', { defaultValue: '生成、上传或从素材库插入内容后，相关素材会出现在这里。' }),
  filteredEmptyTitle: t('creativeStudio.assets.library.filteredEmptyTitle', { defaultValue: '没有匹配的素材' }),
  filteredEmptyDescription: t('creativeStudio.assets.library.filteredEmptyDescription', { defaultValue: '尝试更换类型、范围或搜索关键词。' }),
  retry: t('creativeStudio.assets.library.retry', { defaultValue: '重试' }),
  select: t('creativeStudio.assets.library.select', { defaultValue: '选择素材' }),
  selectAll: t('creativeStudio.assets.library.selectAll', { defaultValue: '全选当前结果' }),
  clearSelection: t('creativeStudio.assets.library.clearSelection', { defaultValue: '取消选择' }),
  selectedCount: (count) => t('creativeStudio.assets.library.selectedCount', { defaultValue: '已选择 {{itemCount}} 项', itemCount: count }),
  resultCount: (visible, total) => t('creativeStudio.assets.library.resultCount', { defaultValue: '已显示 {{visible}} / {{total}} 项', visible, total }),
  addToLibrary: t('creativeStudio.assets.library.addToLibrary', { defaultValue: '加入素材库' }),
  removeFromLibrary: t('creativeStudio.assets.library.removeFromLibrary', { defaultValue: '移出素材库' }),
  insertIntoCanvas: t('creativeStudio.assets.library.insertIntoCanvas', { defaultValue: '插入画布' }),
  downloadSelected: t('creativeStudio.assets.library.downloadSelected', { defaultValue: '下载' }),
  deleteSelected: t('creativeStudio.assets.library.deleteSelected', { defaultValue: '删除' }),
  open: t('creativeStudio.assets.library.open', { defaultValue: '查看' }),
  edit: t('creativeStudio.assets.library.edit', { defaultValue: '编辑' }),
  download: t('creativeStudio.assets.library.download', { defaultValue: '下载' }),
  remove: t('creativeStudio.assets.library.remove', { defaultValue: '删除' }),
  noCollection: t('creativeStudio.assets.library.noCollection', { defaultValue: '未分组' }),
  noTags: t('creativeStudio.assets.library.noTags', { defaultValue: '无标签' }),
  mediaUnavailable: t('creativeStudio.assets.library.mediaUnavailable', { defaultValue: '素材文件不可用' }),
  uploadQueue: t('creativeStudio.assets.library.uploadQueue', { defaultValue: '上传任务' }),
  cancelUpload: t('creativeStudio.assets.library.cancelUpload', { defaultValue: '取消上传' }),
  retryUpload: t('creativeStudio.assets.library.retryUpload', { defaultValue: '重试上传' }),
  dismissUpload: t('creativeStudio.assets.library.dismissUpload', { defaultValue: '移除记录' }),
  uploadComplete: t('creativeStudio.assets.library.uploadComplete', { defaultValue: '上传完成' }),
});

export const createTextAssetLabels = (t: TFunction): CreateTextAssetLabels => ({
  title: t('creativeStudio.assets.textAsset.title', { defaultValue: '新建文本素材' }),
  description: t('creativeStudio.assets.textAsset.description', { defaultValue: '保存可复用的提示文本、文案或创作备注。' }),
  titleLabel: t('creativeStudio.assets.textAsset.titleLabel', { defaultValue: '标题' }),
  titlePlaceholder: t('creativeStudio.assets.textAsset.titlePlaceholder', { defaultValue: '例如：电影感灯光描述' }),
  contentLabel: t('creativeStudio.assets.textAsset.contentLabel', { defaultValue: '文本内容' }),
  contentPlaceholder: t('creativeStudio.assets.textAsset.contentPlaceholder', { defaultValue: '输入要保存的文本内容' }),
  collectionLabel: t('creativeStudio.assets.textAsset.collectionLabel', { defaultValue: '合集' }),
  collectionPlaceholder: t('creativeStudio.assets.textAsset.collectionPlaceholder', { defaultValue: '可选，例如：品牌素材' }),
  tagsLabel: t('creativeStudio.assets.textAsset.tagsLabel', { defaultValue: '标签' }),
  tagsPlaceholder: t('creativeStudio.assets.textAsset.tagsPlaceholder', { defaultValue: '输入标签后按回车' }),
  saveToLibrary: t('creativeStudio.assets.textAsset.saveToLibrary', { defaultValue: '保存到素材库' }),
  cancel: t('creativeStudio.assets.textAsset.cancel', { defaultValue: '取消' }),
  submit: t('creativeStudio.assets.textAsset.submit', { defaultValue: '创建素材' }),
  submitting: t('creativeStudio.assets.textAsset.submitting', { defaultValue: '正在创建' }),
  requiredHint: t('creativeStudio.assets.textAsset.requiredHint', { defaultValue: '标题和文本内容不能为空。' }),
});
