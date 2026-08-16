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

type ResolvedDeliverable = TurnDeliverableItem & { statPath?: string };

/**
 * A reported deliverable may only be presented as available after the backend
 * confirms the file still exists inside an allowed root. Results are cached
 * per app session; a rejected probe is retried on the next mount instead of
 * being cached, so transient transport failures cannot permanently hide a card.
 */
const availabilityCache = new Map<string, Promise<number | null>>();

const probeAvailability = (statPath: string, workspace?: string): Promise<number | null> => {
  const key = `${workspace ?? ''}|${statPath}`;
  const cached = availabilityCache.get(key);
  if (cached) return cached;

  const probe = ipcBridge.fs.getFileMetadata
    .invoke({ path: statPath, workspace })
    .then((metadata) => (metadata && metadata.isDirectory !== true ? metadata.size : null))
    .catch((error) => {
      availabilityCache.delete(key);
      throw error;
    });
  availabilityCache.set(key, probe);
  return probe;
};

const getStatPath = (item: TurnDeliverableItem, workspace?: string): string | undefined => {
  if (item.absolutePath) return item.absolutePath;
  if (workspace) return joinPath(workspace, item.relativePath);
  return undefined;
};

export const useTurnDeliverableAvailability = (
  items: TurnDeliverableItem[],
  workspace?: string
): { pending: boolean; available: ResolvedDeliverable[] } => {
  const [state, setState] = useState<{ key: string; available: ResolvedDeliverable[] } | null>(null);

  const itemsKey = useMemo(
    () => `${workspace ?? ''}|${items.map((item) => `${item.tier}:${item.relativePath}`).join(',')}`,
    [items, workspace]
  );
  // Cards exist only for closed turns, so item content is stable once the key
  // is. Reading through a ref keeps streaming re-renders (new array identity,
  // same key) from re-probing the backend on every token.
  const itemsRef = useRef(items);
  itemsRef.current = items;

  useEffect(() => {
    let alive = true;

    void Promise.all(
      itemsRef.current.map(async (item): Promise<ResolvedDeliverable | null> => {
        const statPath = getStatPath(item, workspace);
        // Committed receipts are already integrity-audited by the backend on
        // both commit and history read; never re-gate them on a client probe.
        if (item.tier === 'receipt') return { ...item, statPath };
        if (!statPath) return null;
        try {
          const size = await probeAvailability(statPath, workspace);
          if (size === null) return null;
          return { ...item, statPath, sizeBytes: item.sizeBytes ?? size };
        } catch {
          return null;
        }
      })
    ).then((resolved) => {
      if (!alive) return;
      setState({
        key: itemsKey,
        available: resolved.filter((item): item is ResolvedDeliverable => item !== null),
      });
    });

    return () => {
      alive = false;
    };
  }, [itemsKey, workspace]);

  if (!state || state.key !== itemsKey) return { pending: true, available: [] };
  return { pending: false, available: state.available };
};

const DeliverableRow: React.FC<{
  item: ResolvedDeliverable;
}> = ({ item }) => {
  const { t } = useTranslation();
  const { launchPreview, canPreview } = usePreviewLauncher();
  const showShellActions = isDesktopShell() && Boolean(item.statPath);
  const displayPath = splitFileDisplayPath(item.relativePath, item.fileName);
  const previewSupported = canPreview && isPreviewSupportedExt(item.fileName);

  const handlePreview = () => {
    const { contentType, editable, language } = getFileTypeInfo(item.fileName);
    void launchPreview({
      relativePath: item.absolutePath ? undefined : item.relativePath,
      originalPath: item.absolutePath ?? item.statPath,
      file_name: item.fileName,
      contentType,
      editable,
      language,
      diffContent: item.diff,
    });
  };

  const handleDiffPreview = () => {
    if (!item.diff) return;
    void launchPreview({
      file_name: item.fileName,
      contentType: 'diff',
      editable: false,
      language: 'diff',
      diffContent: item.diff,
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
        {item.sizeBytes !== undefined && item.sizeBytes > 0 && (
          <span className='text-12px text-t-secondary'>{formatBytes(item.sizeBytes)}</span>
        )}
        {((item.insertions ?? 0) > 0 || (item.deletions ?? 0) > 0) && (
          <span
            className={classNames(
              'flex items-center gap-4px rd-4px px-4px py-2px',
              item.diff && 'cursor-pointer hover:bg-4 transition-colors'
            )}
            title={item.diff ? t('messages.turnDeliverables.viewDiff', { defaultValue: 'View changes' }) : undefined}
            onClick={item.diff ? handleDiffPreview : undefined}
          >
            {(item.insertions ?? 0) > 0 && (
              <span className='text-13px font-medium' style={{ color: diffColors.addition }}>
                +{item.insertions}
              </span>
            )}
            {(item.deletions ?? 0) > 0 && (
              <span className='text-13px font-medium' style={{ color: diffColors.deletion }}>
                -{item.deletions}
              </span>
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

/**
 * Per-turn deliverables card: every verified file artifact of one successfully
 * completed turn, rendered right below that turn's final assistant reply.
 * Renders nothing while availability is being confirmed and nothing when no
 * trustworthy deliverable remains — an empty card must never appear.
 *
 * `partial` marks a turn whose file events may extend past the loaded history
 * window. The card is a projection over hydrated messages, so a turn with more
 * tool events than the window silently lost items after a reload (an observed
 * turn showed 4 files live and 3 after refresh, while the workspace Changes
 * panel — which reads a snapshot rather than messages — still showed 4). A count
 * that cannot be trusted must not be presented as a settled total.
 */
const TurnDeliverablesCard: React.FC<{
  items: TurnDeliverableItem[];
  workspace?: string;
  partial?: boolean;
}> = ({ items, workspace, partial = false }) => {
  const { t } = useTranslation();
  const { pending, available } = useTurnDeliverableAvailability(items, workspace);
  const [showAll, setShowAll] = useState(false);
  const [showAllImages, setShowAllImages] = useState(false);

  if (pending || available.length === 0) return null;

  const verifiedImages = available.filter(isVerifiedImageDeliverable);
  const fileDeliverables = available.filter((item) => !isVerifiedImageDeliverable(item));
  const visibleImages = showAllImages
    ? verifiedImages
    : verifiedImages.slice(0, DEFAULT_VISIBLE_COUNT);
  const hiddenImageCount = verifiedImages.length - visibleImages.length;
  const visible = showAll ? fileDeliverables : fileDeliverables.slice(0, DEFAULT_VISIBLE_COUNT);
  const hiddenCount = fileDeliverables.length - visible.length;

  return (
    <div
      data-testid='turn-deliverables-card'
      data-partial={partial ? 'true' : undefined}
      className='w-full box-border rounded-8px overflow-hidden border border-solid border-[var(--aou-2)]'
    >
      <div className='flex items-center gap-8px px-12px py-8px select-none'>
        <span className='w-8px h-8px rounded-full shrink-0' style={{ backgroundColor: diffColors.addition }}></span>
        <span className='text-14px text-t-primary font-medium'>
          {partial
            ? t('messages.turnDeliverables.partialTitle', {
                count: available.length,
                defaultValue: 'Showing {{count}} changed files from this turn (scroll up to load the rest)',
              })
            : verifiedImages.length === available.length
              ? t('messages.turnDeliverables.imagesTitle', {
                  count: verifiedImages.length,
                  defaultValue: 'Generated {{count}} images',
                })
              : verifiedImages.length > 0
                ? t('messages.turnDeliverables.mixedTitle', {
                    count: available.length,
                    defaultValue: 'Generated {{count}} items',
                  })
                : t('messages.turnDeliverables.title', {
                    count: fileDeliverables.length,
                    defaultValue: 'Generated {{count}} files',
                  })}
        </span>
      </div>

      {verifiedImages.length > 0 && (
        <div
          data-testid='turn-deliverables-images'
          className={classNames(
            'grid grid-cols-1 gap-8px px-12px pb-12px',
            verifiedImages.length > 1 && 'md:grid-cols-2'
          )}
        >
          {visibleImages.map((item) => (
            <VerifiedImageArtifactCard key={item.artifactId} item={item} workspace={workspace} />
          ))}
        </div>
      )}

      {verifiedImages.length > 0 && (hiddenImageCount > 0 || showAllImages) && (
        <button
          type='button'
          aria-expanded={showAllImages}
          className='w-full flex items-center gap-8px px-12px py-6px text-13px text-t-secondary cursor-pointer bg-transparent border-none border-t border-t-solid border-t-[var(--aou-2)] hover:bg-3 transition-colors'
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

      {fileDeliverables.length > 0 && (
        <div className='w-full bg-2'>
          {visible.map((item) => (
            <DeliverableRow key={item.absolutePath ?? item.relativePath} item={item} />
          ))}
          {(hiddenCount > 0 || showAll) && (
            <button
              type='button'
              aria-expanded={showAll}
              className='w-full flex items-center gap-8px px-12px py-6px text-13px text-t-secondary cursor-pointer bg-transparent border-none hover:bg-3 transition-colors'
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
    </div>
  );
};

export default React.memo(TurnDeliverablesCard);
