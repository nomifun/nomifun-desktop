/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { joinPath } from '@/common/chat/chatLib';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { isPreviewSupportedExt } from '@/renderer/pages/conversation/Workspace/utils/filePreview';
import { diffColors, iconColors } from '@/renderer/styles/colors';
import { formatBytes } from '@/renderer/utils/file/formatBytes';
import { getFileTypeInfo } from '@/renderer/utils/file/fileType';
import { splitFileDisplayPath } from '@/renderer/utils/file/pathDisplay';
import { isDesktopShell } from '@/renderer/utils/platform';
import {
  Code,
  DocDetail,
  Down,
  Excel,
  Export,
  FileText,
  FileZip,
  FolderOpen,
  Headset,
  Pic,
  PreviewOpen,
  Shield,
  VideoTwo,
  WebPage,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  isVerifiedImageDeliverable,
  type TurnDeliverableItem,
  type VerifiedImageDeliverableItem,
} from '../turnDeliverablesModel';
import VerifiedImageArtifactCard from './VerifiedImageArtifactCard';

const DEFAULT_VISIBLE_COUNT = 3;
const DIRECTORY_PATH_COLOR = 'color-mix(in srgb, var(--text-secondary) 82%, var(--bg-base))';

const ARCHIVE_EXTENSIONS = new Set(['zip', '7z', 'rar', 'tar', 'gz', 'bz2', 'xz', 'tgz']);

const DeliverableIcon: React.FC<{ fileName: string }> = ({ fileName }) => {
  const iconProps = { theme: 'outline', size: '16', fill: iconColors.secondary } as const;
  const ext = fileName.toLowerCase().split('.').pop() || '';
  if (ARCHIVE_EXTENSIONS.has(ext)) return <FileZip {...iconProps} />;

  switch (getFileTypeInfo(fileName).contentType) {
    case 'image':
      return <Pic {...iconProps} />;
    case 'html':
      return <WebPage {...iconProps} />;
    case 'markdown':
      return <FileText {...iconProps} />;
    case 'pdf':
    case 'word':
    case 'ppt':
      return <DocDetail {...iconProps} />;
    case 'excel':
      return <Excel {...iconProps} />;
    default:
      if (ext === 'mp3' || ext === 'wav' || ext === 'flac' || ext === 'ogg' || ext === 'm4a') {
        return <Headset {...iconProps} />;
      }
      if (ext === 'mp4' || ext === 'mov' || ext === 'webm' || ext === 'avi' || ext === 'mkv') {
        return <VideoTwo {...iconProps} />;
      }
      return <Code {...iconProps} />;
  }
};

type ResolvedDeliverable = TurnDeliverableItem & { statPath?: string; available: boolean };

// All mounted Turn cards share the same small read budget. This is only an
// in-flight scheduler; no positive or negative filesystem result is cached.
let activeMetadataProbes = 0;
const metadataProbeQueue: Array<() => void> = [];
const drainMetadataProbes = () => {
  while (activeMetadataProbes < 4 && metadataProbeQueue.length) metadataProbeQueue.shift()?.();
};
const probeMetadataAvailability = (
  path: string,
  workspace: string | undefined,
  isCurrent: () => boolean
): Promise<boolean> => new Promise((resolve) => {
  metadataProbeQueue.push(() => {
    if (!isCurrent()) { resolve(false); return; }
    activeMetadataProbes += 1;
    void ipcBridge.fs.getFileMetadata.invoke({ path, workspace })
      .then((metadata) => resolve(Boolean(metadata && metadata.isDirectory !== true)))
      .catch(() => resolve(false))
      .finally(() => { activeMetadataProbes -= 1; drainMetadataProbes(); });
  });
  drainMetadataProbes();
});

const getStatPath = (item: TurnDeliverableItem, workspace?: string): string | undefined => {
  if (item.operation === 'delete') return undefined;
  if (item.absolutePath) return item.absolutePath;
  // Canonical file queries may deliberately omit a locator when the historic
  // workspace cannot be resolved. Never guess a same-name current-workspace file.
  if (isVerifiedImageDeliverable(item) && workspace) return joinPath(workspace, item.relativePath);
  return undefined;
};

/** Availability controls actions only. Historical changes never disappear. */
export const useTurnDeliverableAvailability = (
  items: TurnDeliverableItem[],
  workspace?: string,
  visibleFileCount = DEFAULT_VISIBLE_COUNT
): ResolvedDeliverable[] => {
  const [state, setState] = useState<{ key: string; items: ResolvedDeliverable[] } | null>(null);
  const itemsKey = useMemo(
    () => JSON.stringify([workspace, visibleFileCount, items.map((item) => [item.identity, item.absolutePath, item.operation, item.sizeBytes, item.sha256, item.tier, item.artifactId, item.artifactKind, item.mimeType])]),
    [items, workspace, visibleFileCount]
  );
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const initial = () => itemsRef.current.map((item): ResolvedDeliverable => ({
    ...item, statPath: getStatPath(item, workspace), available: isVerifiedImageDeliverable(item),
  }));

  useEffect(() => {
    let alive = true;
    let generation = 0;
    const refresh = () => {
      const currentGeneration = ++generation;
      const resolved = initial();
      const targets = resolved.map((item, index) => ({ item, index }))
        .filter(({ item }) => !isVerifiedImageDeliverable(item)).slice(0, visibleFileCount);
      let next = 0;
      // Hidden files need no filesystem requests. Expanded lists use a small
      // worker pool, so a large Turn cannot flood the host's metadata route.
      const probe = async () => {
        while (alive && currentGeneration === generation && next < targets.length) {
          const { item, index } = targets[next++];
          if (!item.statPath) continue;
          const available = await probeMetadataAvailability(item.statPath, workspace,
            () => alive && currentGeneration === generation);
          if (alive && currentGeneration === generation) {
            resolved[index] = { ...item, available };
            setState({ key: itemsKey, items: [...resolved] });
          }
        }
      };
      void Promise.all(Array.from({ length: Math.min(4, targets.length) }, probe));
      setState({ key: itemsKey, items: [...resolved] });
    };
    window.addEventListener('focus', refresh);
    refresh();
    return () => {
      alive = false;
      window.removeEventListener('focus', refresh);
    };
  }, [itemsKey, workspace, visibleFileCount]);

  return state?.key === itemsKey ? state.items : initial();
};

const DeliverableRow: React.FC<{
  item: ResolvedDeliverable;
}> = ({ item }) => {
  const { t } = useTranslation();
  const { launchPreview, canPreview } = usePreviewLauncher();
  const showShellActions = isDesktopShell() && item.available && Boolean(item.statPath);
  const displayPath = splitFileDisplayPath(item.relativePath, item.fileName);
  const previewSupported = item.available && canPreview && isPreviewSupportedExt(item.fileName);

  const handlePreview = () => {
    const { contentType, editable, language } = getFileTypeInfo(item.fileName);
    void launchPreview({
      relativePath: item.absolutePath ? undefined : item.relativePath,
      originalPath: item.absolutePath ?? item.statPath,
      file_name: item.fileName,
      contentType,
      editable,
      language,
    });
  };

  return (
    <div
      data-deliverable-path={item.relativePath}
      className='group flex items-center justify-between gap-8px px-12px py-6px hover:bg-3 transition-colors'
    >
      <div className='flex flex-1 items-center gap-8px min-w-0'>
        <span className='shrink-0 flex items-center' style={{ lineHeight: 0 }}>
          <DeliverableIcon fileName={item.fileName} />
        </span>
        <span
          className='flex min-w-0 max-w-full items-center text-14px leading-20px'
          title={displayPath.fullPath}
        >
          {displayPath.directoryPath && (
            <span className='min-w-0 truncate' style={{ color: DIRECTORY_PATH_COLOR }}>
              {displayPath.directoryPath}
            </span>
          )}
          <span
            className={classNames(
              'truncate text-t-primary',
              displayPath.directoryPath ? 'shrink-0 max-w-60%' : 'min-w-0'
            )}
          >
            {displayPath.fileName}
          </span>
        </span>
        {item.tier === 'receipt' && (
          <span
            className='shrink-0 flex items-center'
            style={{ lineHeight: 0 }}
            title={t('messages.turnDeliverables.verified', { defaultValue: 'Integrity verified (SHA-256)' })}
          >
            <Shield theme='outline' size='13' fill={diffColors.addition} />
          </span>
        )}
      </div>

      <div className='flex items-center gap-8px shrink-0'>
        {(item.operation || item.sizeBytes !== undefined) && (
          <span className='text-12px text-t-secondary'>
            {item.operation && t(`messages.turnDeliverables.${item.operation}`)}
            {item.operation !== 'delete' && item.sizeBytes !== undefined && (
              <>{item.operation ? ' · ' : ''}{formatBytes(item.sizeBytes)}</>
            )}
          </span>
        )}
        {previewSupported && (
          <span
            className='flex items-center gap-4px text-12px text-t-secondary cursor-pointer rd-4px px-4px py-2px hover:bg-4'
            onClick={handlePreview}
          >
            <PreviewOpen className='line-height-8px' theme='outline' size='14' fill={iconColors.secondary} />
            {t('preview.preview')}
          </span>
        )}
        {showShellActions && (
          <>
            <span
              className='flex items-center cursor-pointer rd-4px p-2px hover:bg-4'
              style={{ lineHeight: 0 }}
              title={t('messages.turnDeliverables.open', { defaultValue: 'Open' })}
              onClick={() => void ipcBridge.shell.openFile.invoke(item.statPath!)}
            >
              <Export theme='outline' size='14' fill={iconColors.secondary} />
            </span>
            <span
              className='flex items-center cursor-pointer rd-4px p-2px hover:bg-4'
              style={{ lineHeight: 0 }}
              title={t('messages.turnDeliverables.reveal', { defaultValue: 'Reveal in folder' })}
              onClick={() => void ipcBridge.shell.showItemInFolder.invoke(item.statPath!)}
            >
              <FolderOpen theme='outline' size='14' fill={iconColors.secondary} />
            </span>
          </>
        )}
      </div>
    </div>
  );
};

/** Turn-scoped file facts render in the existing list and verified media layout. */
const TurnDeliverablesCard: React.FC<{
  items: TurnDeliverableItem[];
  workspace?: string;
  partial?: boolean;
}> = ({ items, workspace, partial = false }) => {
  const { t } = useTranslation();
  const [showAll, setShowAll] = useState(false);
  const resolved = useTurnDeliverableAvailability(items, workspace, showAll ? items.length : DEFAULT_VISIBLE_COUNT);
  const [showAllImages, setShowAllImages] = useState(false);
  const verifiedImages = resolved.filter((item): item is ResolvedDeliverable & VerifiedImageDeliverableItem => isVerifiedImageDeliverable(item));
  const fileDeliverables = resolved.filter((item) => !isVerifiedImageDeliverable(item));
  const imageIdentity = verifiedImages.map((item) => item.artifactId).join('|');
  const [selectedImageId, setSelectedImageId] = useState<string | undefined>();

  useEffect(() => {
    if (!verifiedImages.length) {
      setSelectedImageId(undefined);
      return;
    }
    setSelectedImageId((current) =>
      current && verifiedImages.some((item) => item.artifactId === current)
        ? current
        : verifiedImages[0].artifactId
    );
  }, [imageIdentity]);

  if (resolved.length === 0 && !partial) return null;

  const visibleImages = showAllImages
    ? verifiedImages
    : verifiedImages.slice(0, DEFAULT_VISIBLE_COUNT);
  const hiddenImageCount = verifiedImages.length - visibleImages.length;
  const selectedImage = verifiedImages.find((item) => item.artifactId === selectedImageId) ?? verifiedImages[0];
  const visible = showAll ? fileDeliverables : fileDeliverables.slice(0, DEFAULT_VISIBLE_COUNT);
  const hiddenCount = fileDeliverables.length - visible.length;

  return (
    <section
      data-testid='turn-deliverables-card'
      data-partial={partial ? 'true' : undefined}
      className='turn-deliverables'
    >
      {selectedImage && (
        <div
          data-testid='turn-deliverables-images'
          className='turn-deliverables__media-layout'
        >
          <VerifiedImageArtifactCard
            key={selectedImage.artifactId}
            item={selectedImage}
            workspace={workspace}
          />
          {verifiedImages.length > 1 && (
            <div className='turn-deliverables__thumbnail-rail' aria-label={t('messages.turnDeliverables.imagesTitle', {
              count: verifiedImages.length,
              defaultValue: 'Generated {{count}} images',
            })}>
              {visibleImages.map((item) => (
                <VerifiedImageArtifactCard
                  key={item.artifactId}
                  item={item}
                  workspace={workspace}
                  variant='thumbnail'
                  selected={item.artifactId === selectedImage.artifactId}
                  onSelect={() => setSelectedImageId(item.artifactId)}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {verifiedImages.length > 0 && (hiddenImageCount > 0 || showAllImages) && (
        <button
          type='button'
          aria-expanded={showAllImages}
          className='turn-deliverables__more'
          onClick={() => setShowAllImages(!showAllImages)}
        >
          <Down
            theme='outline'
            size='14'
            fill={iconColors.secondary}
            className={classNames('transition-transform duration-200', showAllImages && 'rotate-180')}
          />
          {showAllImages
            ? t('messages.turnDeliverables.showLessImages', { defaultValue: 'Show fewer images' })
            : t('messages.turnDeliverables.showMoreImages', {
                count: hiddenImageCount,
                defaultValue: 'Show {{count}} more images',
              })}
        </button>
      )}

      {(fileDeliverables.length > 0 || partial) && (
        <div className='turn-deliverables__files'>
          <div className='turn-deliverables__files-title'>
            {partial
              ? t('messages.turnDeliverables.partialTitle', {
                  count: fileDeliverables.length,
                  defaultValue: 'This turn involves {{count}} files (list incomplete)',
                })
              : t('messages.turnDeliverables.title', {
                  count: fileDeliverables.length,
                  defaultValue: 'This turn involves {{count}} files',
                })}
          </div>
          {visible.map((item) => (
            <DeliverableRow key={item.identity} item={item} />
          ))}
          {(hiddenCount > 0 || showAll) && (
            <button
              type='button'
              aria-expanded={showAll}
              className='turn-deliverables__more'
              onClick={() => setShowAll(!showAll)}
            >
              <Down
                theme='outline'
                size='14'
                fill={iconColors.secondary}
                className={classNames('transition-transform duration-200', showAll && 'rotate-180')}
              />
              {showAll
                ? t('messages.turnDeliverables.showLess', { defaultValue: 'Show less' })
                : t('messages.turnDeliverables.showMore', {
                    count: hiddenCount,
                    defaultValue: 'Show {{count}} more files',
                  })}
            </button>
          )}
        </div>
      )}
    </section>
  );
};

export default React.memo(TurnDeliverablesCard);
