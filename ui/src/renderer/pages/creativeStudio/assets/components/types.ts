/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

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

export const DEFAULT_CREATIVE_ASSET_LIBRARY_LABELS: CreativeAssetLibraryLabels = {
  title: '素材库',
  description: '集中管理画布使用的图片、视频、音频与文本素材。',
  searchPlaceholder: '搜索标题、合集或标签',
  clearSearch: '清除搜索',
  kindFilter: '素材类型',
  scopeFilter: '素材范围',
  viewFilter: '显示方式',
  all: '全部',
  image: '图片',
  video: '视频',
  audio: '音频',
  text: '文本',
  libraryScope: '素材库',
  canvasScope: '画布素材',
  gridView: '网格视图',
  listView: '列表视图',
  upload: '上传素材',
  createText: '新建文本',
  renameCollection: '重命名合集',
  search: '搜索',
  pagination: '素材分页',
  previousPage: '上一页',
  nextPage: '下一页',
  pageSize: (size) => `${size} 条/页`,
  dropFiles: '释放文件以上传',
  loading: '正在加载素材',
  loadingMore: '正在加载更多',
  loadMore: '加载更多',
  emptyTitle: '还没有素材',
  emptyDescription: '上传媒体或创建文本素材，开始构建你的创作资源库。',
  canvasEmptyTitle: '当前画布还没有素材',
  canvasEmptyDescription: '生成、上传或从素材库插入内容后，相关素材会出现在这里。',
  filteredEmptyTitle: '没有匹配的素材',
  filteredEmptyDescription: '尝试更换类型、范围或搜索关键词。',
  retry: '重试',
  select: '选择素材',
  selectAll: '全选当前结果',
  clearSelection: '取消选择',
  selectedCount: (count) => `已选择 ${count} 项`,
  resultCount: (visible, total) => `已显示 ${visible} / ${total} 项`,
  addToLibrary: '加入素材库',
  removeFromLibrary: '移出素材库',
  insertIntoCanvas: '插入画布',
  downloadSelected: '下载',
  deleteSelected: '删除',
  open: '查看',
  edit: '编辑',
  download: '下载',
  remove: '删除',
  noCollection: '未分组',
  noTags: '无标签',
  mediaUnavailable: '素材文件不可用',
  uploadQueue: '上传任务',
  cancelUpload: '取消上传',
  retryUpload: '重试上传',
  dismissUpload: '移除记录',
  uploadComplete: '上传完成',
};

export const DEFAULT_CREATE_TEXT_ASSET_LABELS: CreateTextAssetLabels = {
  title: '新建文本素材',
  description: '保存可复用的提示文本、文案或创作备注。',
  titleLabel: '标题',
  titlePlaceholder: '例如：电影感灯光描述',
  contentLabel: '文本内容',
  contentPlaceholder: '输入要保存的文本内容',
  collectionLabel: '合集',
  collectionPlaceholder: '可选，例如：品牌素材',
  tagsLabel: '标签',
  tagsPlaceholder: '输入标签后按回车',
  saveToLibrary: '保存到素材库',
  cancel: '取消',
  submit: '创建素材',
  submitting: '正在创建',
  requiredHint: '标题和文本内容不能为空。',
};
