/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Tag } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import {
  CreativeDetailFacts,
  CreativeDetailLayout,
  CreativeDetailModal,
  CreativeDetailSection,
  CreativeDetailText,
} from '../../components/CreativeDetailLayout';
import CreativeVideoPlayer from '../components/CreativeVideoPlayer';
import { creativeAssetDisplayTitle, creativeAssetTags, formatCreativeAssetBytes } from '../presentation';
import { isCreativeAssetDeleted, type CreativeAsset } from '../types';
import styles from './CreativeAssetPreviewModal.module.css';

interface CreativeAssetPreviewModalProps {
  asset: CreativeAsset | null;
  locale?: string;
  onClose: () => void;
  onDownload: (asset: CreativeAsset) => void;
}

const formatDate = (timestamp: number, locale: string): string => {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '—';
  try {
    return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
  } catch {
    return date.toISOString();
  }
};

const CreativeAssetPreviewModal: React.FC<CreativeAssetPreviewModalProps> = ({
  asset, locale, onClose, onDownload,
}) => {
  const { t, i18n } = useTranslation();
  const title = asset ? creativeAssetDisplayTitle(asset) : '';
  const tags = asset ? creativeAssetTags(asset) : [];
  const collection = asset?.collection?.trim();
  const prompt = asset?.origin?.prompt;
  const deleted = Boolean(asset && isCreativeAssetDeleted(asset));
  const kindLabels = {
    image: t('creativeStudio.assets.kind.image', { defaultValue: '图片' }),
    video: t('creativeStudio.assets.kind.video', { defaultValue: '视频' }),
    audio: t('creativeStudio.assets.kind.audio', { defaultValue: '音频' }),
    text: t('creativeStudio.assets.kind.text', { defaultValue: '文本' }),
  };

  return (
    <CreativeDetailModal
      visible={Boolean(asset)}
      title={t('creativeStudio.assets.preview.title', { defaultValue: '素材详情' })}
      onClose={onClose}
    >
      {asset ? (
        <CreativeDetailLayout
          data-creative-asset-preview={asset.kind}
          visual={(
            <div className={styles.previewMedia}>
              {deleted ? (
                <p role='status'>{t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })}</p>
              ) : asset.kind === 'image' ? (
                <img src={asset.originalUrl} alt={title} />
              ) : asset.kind === 'video' ? (
                <div className={styles.previewVideo}>
                  <CreativeVideoPlayer src={asset.originalUrl} poster={asset.thumbnailUrl ?? undefined} label={title} />
                </div>
              ) : asset.kind === 'audio' ? (
                <audio src={asset.originalUrl} controls preload='metadata' aria-label={title} />
              ) : (
                <pre className={styles.previewText}>{asset.textContent ?? ''}</pre>
              )}
            </div>
          )}
          badges={<Tag color={asset.kind === 'image' ? 'green' : asset.kind === 'video' ? 'orange' : 'arcoblue'}>{kindLabels[asset.kind]}</Tag>}
          footer={(
            <>
              {asset.kind !== 'text' ? (
                <Button type='primary' disabled={deleted} onClick={() => { if (!deleted) onDownload(asset); }}>
                  {t('creativeStudio.assets.preview.downloadOriginal', { defaultValue: '下载原始文件' })}
                </Button>
              ) : null}
              <Button onClick={onClose}>{t('creativeStudio.assets.preview.close', { defaultValue: '关闭' })}</Button>
            </>
          )}
        >
          <CreativeDetailSection
            label={t('creativeStudio.assets.edit.titleLabel', { defaultValue: '标题' })}
            action={!deleted && asset.kind === 'text' && asset.textContent ? <CopyIconButton text={asset.textContent} /> : null}
          >
            <h3 className={styles.previewTitle}>{title}</h3>
          </CreativeDetailSection>

          {prompt?.trim() ? (
            <CreativeDetailSection
              label={t('creativeStudio.image.results.detailPrompt', { defaultValue: '完整提示词' })}
              action={(
                <CopyIconButton
                  text={prompt}
                  tooltip={t('creativeStudio.image.results.copyPrompt', { defaultValue: '复制提示词' })}
                  successMessage={t('creativeStudio.image.results.promptCopied', { defaultValue: '提示词已复制' })}
                  size={14}
                />
              )}
            >
              <CreativeDetailText>{prompt}</CreativeDetailText>
            </CreativeDetailSection>
          ) : null}

          <CreativeDetailFacts>
            {asset.origin?.providerId ? <div>
              <dt>{t('creativeStudio.image.results.detailProvider', { defaultValue: '提供商' })}</dt>
              <dd>{asset.origin.providerId}</dd>
            </div> : null}
            {asset.origin?.model ? <div>
              <dt>{t('creativeStudio.image.results.detailModel', { defaultValue: '模型' })}</dt>
              <dd>{asset.origin.model}</dd>
            </div> : null}
            {collection ? <div>
              <dt>{t('creativeStudio.assets.edit.collectionLabel', { defaultValue: '合集' })}</dt>
              <dd>{collection}</dd>
            </div> : null}
            {asset.width && asset.height ? <div>
              <dt>{t('creativeStudio.assets.preview.dimensions', { defaultValue: '尺寸' })}</dt>
              <dd>{asset.width} × {asset.height}</dd>
            </div> : null}
            {asset.bytes !== null && Number.isFinite(asset.bytes) && asset.bytes >= 0 ? <div>
              <dt>{t('creativeStudio.assets.preview.fileSize', { defaultValue: '文件大小' })}</dt>
              <dd>{formatCreativeAssetBytes(asset.bytes)}</dd>
            </div> : null}
            {asset.mimeType ? <div>
              <dt>{t('creativeStudio.assets.preview.format', { defaultValue: '文件格式' })}</dt>
              <dd>{asset.mimeType}</dd>
            </div> : null}
            <div>
              <dt>{t('creativeStudio.assets.preview.createdAt', { defaultValue: '创建时间' })}</dt>
              <dd>{formatDate(asset.createdAt, locale ?? i18n.language)}</dd>
            </div>
            <div>
              <dt>{t('creativeStudio.assets.preview.updatedAt', { defaultValue: '更新时间' })}</dt>
              <dd>{formatDate(asset.updatedAt, locale ?? i18n.language)}</dd>
            </div>
          </CreativeDetailFacts>

          {tags.length ? (
            <CreativeDetailSection label={t('creativeStudio.assets.preview.tags', { defaultValue: '素材标签' })}>
              <div className={styles.previewTags}>
                {tags.map((tag) => <span key={tag}>{tag}</span>)}
              </div>
            </CreativeDetailSection>
          ) : null}
        </CreativeDetailLayout>
      ) : null}
    </CreativeDetailModal>
  );
};

export default CreativeAssetPreviewModal;
