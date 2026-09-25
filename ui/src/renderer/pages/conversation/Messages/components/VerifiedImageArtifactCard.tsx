/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import LocalImageView from '@/renderer/components/media/LocalImageView';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { diffColors, iconColors } from '@/renderer/styles/colors';
import { downloadFileFromPath } from '@/renderer/utils/file/download';
import { formatBytes } from '@/renderer/utils/file/formatBytes';
import { isDesktopShell } from '@/renderer/utils/platform';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { Message } from '@arco-design/web-react';
import { Copy, Download, PreviewOpen, Shield } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { VerifiedImageDeliverableItem } from '../turnDeliverablesModel';

type ResolvedVerifiedImage = VerifiedImageDeliverableItem & { statPath?: string };

interface VerifiedImageArtifactCardProps {
  item: ResolvedVerifiedImage;
  workspace?: string;
  variant?: 'primary' | 'thumbnail';
  selected?: boolean;
  onSelect?: () => void;
}

/**
 * First-class image result for one backend-verified, durably committed artifact.
 * The caller can only construct this prop through isVerifiedImageDeliverable;
 * no assistant Markdown URL or filename-extension inference enters this view.
 */
const VerifiedImageArtifactCard: React.FC<VerifiedImageArtifactCardProps> = ({
  item,
  workspace,
  variant = 'primary',
  selected = false,
  onSelect,
}) => {
  const { t } = useTranslation();
  const { launchPreview, canPreview } = usePreviewLauncher();
  const desktop = isDesktopShell();
  const imagePath = item.absolutePath ?? item.statPath ?? item.relativePath;
  const canOpen = canPreview || (desktop && Boolean(item.statPath));
  const canDownload = Boolean(item.statPath);
  const copyTarget = useMemo(
    () => (desktop ? item.statPath ?? item.absolutePath ?? item.relativePath : item.relativePath),
    [desktop, item.absolutePath, item.relativePath, item.statPath]
  );

  const handleOpen = () => {
    if (canPreview) {
      void launchPreview({
        relativePath: item.absolutePath ? undefined : item.relativePath,
        originalPath: item.absolutePath ?? item.statPath,
        file_name: item.fileName,
        title: item.fileName,
        contentType: 'image',
        editable: false,
      });
      return;
    }
    if (desktop && item.statPath) {
      void ipcBridge.shell.openFile.invoke(item.statPath).catch(() => {
        Message.error(t('preview.openInSystemFailed'));
      });
    }
  };

  const handleDownload = async () => {
    if (!item.statPath) return;
    try {
      await downloadFileFromPath(item.statPath, item.fileName, workspace);
      Message.success(t('messages.downloadSuccess'));
    } catch {
      Message.error(t('messages.downloadFailed'));
    }
  };

  const handleCopyPath = async () => {
    try {
      await copyText(copyTarget);
      Message.success(t('messages.copySuccess'));
    } catch {
      Message.error(t('messages.copyFailed'));
    }
  };

  const openLabel = t('messages.turnDeliverables.openImage', { defaultValue: 'Open image' });
  const saveLabel = t('messages.turnDeliverables.saveImage', { defaultValue: 'Save image' });
  const copyLabel = t('messages.turnDeliverables.copyImagePath', { defaultValue: 'Copy image path' });

  if (variant === 'thumbnail') {
    return (
      <button
        type='button'
        data-testid='verified-image-artifact-thumbnail'
        data-artifact-id={item.artifactId}
        aria-pressed={selected}
        aria-label={t('messages.turnDeliverables.selectImage', {
          name: item.fileName,
          defaultValue: 'Select {{name}}',
        })}
        className='turn-deliverable-image__thumbnail'
        onClick={onSelect}
      >
        <LocalImageView
          src={imagePath}
          alt={item.fileName}
          className='turn-deliverable-image__thumbnail-media'
        />
      </button>
    );
  }

  return (
    <article
      data-testid='verified-image-artifact-card'
      data-artifact-id={item.artifactId}
      className='turn-deliverable-image'
    >
      <button
        type='button'
        aria-label={openLabel}
        disabled={!canOpen}
        className='turn-deliverable-image__preview'
        onClick={handleOpen}
      >
        <LocalImageView
          src={imagePath}
          alt={item.fileName}
          className='turn-deliverable-image__media'
        />
      </button>

      <div className='turn-deliverable-image__meta'>
        <div className='flex min-w-120px flex-1 items-center gap-6px'>
          <Shield
            theme='outline'
            size='13'
            fill={diffColors.addition}
            aria-label={t('messages.turnDeliverables.verified', {
              defaultValue: 'Integrity verified (SHA-256)',
            })}
          />
          <span className='min-w-0 truncate text-13px text-t-primary' title={item.relativePath}>
            {item.fileName}
          </span>
          {item.sizeBytes !== undefined && item.sizeBytes > 0 && (
            <span className='shrink-0 text-12px text-t-secondary'>{formatBytes(item.sizeBytes)}</span>
          )}
        </div>

        <div className='turn-deliverable-image__actions'>
          {canOpen && (
            <button
              type='button'
              aria-label={openLabel}
              title={openLabel}
              className='inline-flex items-center gap-4px rd-4px border-none bg-transparent px-6px py-4px text-12px text-t-secondary cursor-pointer hover:bg-4 hover:text-t-primary'
              onClick={handleOpen}
            >
              <PreviewOpen theme='outline' size='14' fill={iconColors.secondary} />
              <span>{openLabel}</span>
            </button>
          )}
          {canDownload && (
            <button
              type='button'
              aria-label={saveLabel}
              title={saveLabel}
              className='inline-flex items-center gap-4px rd-4px border-none bg-transparent px-6px py-4px text-12px text-t-secondary cursor-pointer hover:bg-4 hover:text-t-primary'
              onClick={() => void handleDownload()}
            >
              <Download theme='outline' size='14' fill={iconColors.secondary} />
              <span>{saveLabel}</span>
            </button>
          )}
          <button
            type='button'
            aria-label={copyLabel}
            title={`${copyLabel}: ${copyTarget}`}
            className='inline-flex items-center gap-4px rd-4px border-none bg-transparent px-6px py-4px text-12px text-t-secondary cursor-pointer hover:bg-4 hover:text-t-primary'
            onClick={() => void handleCopyPath()}
          >
            <Copy theme='outline' size='14' fill={iconColors.secondary} />
            <span>{copyLabel}</span>
          </button>
        </div>
      </div>
    </article>
  );
};

export default React.memo(VerifiedImageArtifactCard);
