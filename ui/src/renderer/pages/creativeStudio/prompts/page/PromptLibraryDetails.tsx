/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Modal } from '@arco-design/web-react';
import { Copy, FolderPlus } from '@icon-park/react';
import React from 'react';

import { openExternalUrl } from '@/renderer/utils/platform';

import type { PromptLibraryItem } from '../types';
import styles from './PromptLibraryDetails.module.css';

export type PromptCopyState = 'idle' | 'copying' | 'copied' | 'failed';
export type PromptSaveState = 'idle' | 'saving' | 'saved' | 'failed';

export interface PromptLibraryDetailsProps {
  item: PromptLibraryItem | null;
  locale: string;
  copyState: PromptCopyState;
  copyError?: string | null;
  saveState?: PromptSaveState;
  saveError?: string | null;
  onClose(): void;
  onCopy(): void;
  onSave?(): void;
}

export interface PromptLibraryDetailsContentProps {
  item: PromptLibraryItem;
  locale: string;
}

function sourceLabel(item: PromptLibraryItem): string {
  if (item.source === 'catalog') return '公共提示词目录';
  return item.source === 'preset' ? 'NomiFun 预设' : '我的文本素材';
}

function updatedAtLabel(value: number | null, locale: string): string | null {
  if (value === null) return null;
  const milliseconds = value < 10_000_000_000 ? value * 1_000 : value;
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

function openAuditableSource(event: React.MouseEvent<HTMLAnchorElement>): void {
  event.preventDefault();
  const url = event.currentTarget.href;
  void openExternalUrl(url).catch(() => Message.error('无法打开外部链接'));
}

export const PromptLibraryDetailsContent: React.FC<PromptLibraryDetailsContentProps> = ({
  item,
  locale,
}) => {
  const updatedAt = updatedAtLabel(item.updatedAt, locale);
  const preview = item.preview?.replace(/!\[[^\]]*]\([^)]+\)/g, '').trim() ?? '';
  return (
    <div
      className={styles.content}
      data-prompt-library-details='true'
      data-prompt-source={item.source}
    >
      <div className={styles.detailsGrid} data-has-cover={item.coverUrl ? 'true' : 'false'}>
        {item.coverUrl ? (
          <div className={styles.previewColumn}>
            <img
              className={styles.cover}
              src={item.coverUrl}
              alt={item.title}
              referrerPolicy='no-referrer'
            />
            {preview ? <pre className={styles.preview}>{preview}</pre> : null}
          </div>
        ) : null}

        <div className={styles.promptColumn}>
          <div className={styles.metadata}>
            <span className={styles.source}>{sourceLabel(item)}</span>
            <span className={styles.category}>{item.category ?? '未分类'}</span>
          </div>

          {item.description ? <p className={styles.description}>{item.description}</p> : null}

          <div className={styles.promptBlock}>
            <span className={styles.sectionLabel}>完整提示词</span>
            <pre className={styles.prompt}>{item.prompt}</pre>
          </div>

          {item.tags.length > 0 ? (
            <div className={styles.tagList} aria-label='提示词标签'>
              {item.tags.map((tag) => (
                <span key={tag} className={styles.tag}>
                  {tag}
                </span>
              ))}
            </div>
          ) : null}

          {updatedAt || item.knowledgeBaseIds.length > 0 || item.license ? (
            <div className={styles.facts}>
              {updatedAt ? <span>更新于 {updatedAt}</span> : null}
              {item.knowledgeBaseIds.length > 0 ? (
                <span>关联 {item.knowledgeBaseIds.length} 个知识库</span>
              ) : null}
              {item.sourceUrl ? (
                <a
                  href={item.sourceUrl}
                  target='_blank'
                  rel='noreferrer'
                  onClick={openAuditableSource}
                >
                  查看来源
                </a>
              ) : null}
              {item.license ? (
                item.licenseUrl ? (
                  <a
                    href={item.licenseUrl}
                    target='_blank'
                    rel='noreferrer'
                    onClick={openAuditableSource}
                  >
                    {item.license}
                  </a>
                ) : (
                  <span>{item.license}</span>
                )
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
};

export const PromptLibraryDetails: React.FC<PromptLibraryDetailsProps> = ({
  item,
  locale,
  copyState,
  copyError,
  saveState = 'idle',
  saveError,
  onClose,
  onCopy,
  onSave,
}) => (
  <Modal
    visible={item !== null}
    title={item?.title ?? '提示词详情'}
    footer={null}
    style={{ width: 860, maxWidth: 'calc(100vw - 32px)' }}
    autoFocus={false}
    unmountOnExit
    getPopupContainer={() =>
      document.getElementById('creative-studio-portal-root') ?? document.body
    }
    onCancel={onClose}
  >
    {item ? (
      <>
        <PromptLibraryDetailsContent item={item} locale={locale} />
        <div className={styles.actions}>
          <p
            className={styles.copyFeedback}
            data-copy-state={copyState}
            role={copyState === 'failed' || saveState === 'failed' ? 'alert' : 'status'}
            aria-live='polite'
          >
            {saveState === 'saved'
              ? '已加入“我的素材”。'
              : saveState === 'failed'
                ? saveError || '保存失败，请稍后重试。'
                : copyState === 'copied'
              ? '提示词已复制到剪贴板。'
              : copyState === 'failed'
                ? copyError || '复制失败，请检查剪贴板权限。'
                : '独立提示词库不会修改任何画布。'}
          </p>
          <div className={styles.actionButtons}>
            <Button
              type='primary'
              icon={<Copy theme='outline' size={15} fill='currentColor' />}
              loading={copyState === 'copying'}
              onClick={onCopy}
            >
              复制提示词
            </Button>
            {onSave ? (
              <Button
                icon={<FolderPlus theme='outline' size={15} fill='currentColor' />}
                loading={saveState === 'saving'}
                disabled={saveState === 'saved'}
                onClick={onSave}
              >
                {saveState === 'saved' ? '已加入素材' : '加入我的素材'}
              </Button>
            ) : null}
          </div>
        </div>
      </>
    ) : null}
  </Modal>
);

export default PromptLibraryDetails;
