/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Camera,
  PanoramaHorizontal,
  Pic,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeCanvasNodeKind } from '../../domain/schema';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import CreativeNodeFrame from './CreativeNodeFrame';
import CreativeVideoNodeMedia from './CreativeVideoNodeMedia';
import type {
  CreativeNodeAssetPresentation,
  CreativeNodeOfKind,
  CreativeNodePresentationProps,
} from './types';
import styles from './CreativeNodeViews.module.css';

interface EmptyMediaProps {
  icon: React.ReactNode;
  label: string;
  assetId?: string | null;
}

const EmptyMedia: React.FC<EmptyMediaProps> = ({ icon, label, assetId }) => (
  <div className={styles.emptyMedia} data-node-empty-media>
    <span className={styles.emptyIcon} aria-hidden='true'>
      {icon}
    </span>
    <strong>{label}</strong>
    {assetId ? <span className={styles.assetReference}>{assetId}</span> : null}
  </div>
);

const formatMilliseconds = (milliseconds: number) => {
  const safeMilliseconds = Math.max(0, Number.isFinite(milliseconds) ? milliseconds : 0);
  const totalSeconds = Math.floor(safeMilliseconds / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
};

/**
 * Helper kept concrete at call sites so canonical discriminated-union types
 * remain visible to TypeScript. It does not copy or persist node fields.
 */
const nodeCallbacks = <K extends CreativeCanvasNodeKind>(
  node: CreativeNodeOfKind<K>,
  props: CreativeNodePresentationProps<K>
) => ({
  onActivate: props.onActivate ? () => props.onActivate?.(node) : undefined,
  onOpen: props.onOpen ? () => props.onOpen?.(node) : undefined,
  onToggleLock: props.onToggleLock ? () => props.onToggleLock?.(node) : undefined,
});

const sharedFrameProps = <K extends CreativeCanvasNodeKind>(props: CreativeNodePresentationProps<K>) => ({
  selected: props.selected,
  placement: props.placement,
  runtime: props.runtime,
  className: props.className,
  style: props.style,
  headerActions: props.headerActions,
  inputHandle: props.inputHandle,
  outputHandle: props.outputHandle,
  onPointerDown: props.onPointerDown,
  onContextMenu: props.onContextMenu,
  ...nodeCallbacks(props.node, props),
});

export interface CreativeTextNodeProps extends CreativeNodePresentationProps<'text'> {
  title?: string;
  emptyLabel?: string;
  editing?: boolean;
  onTextChange?: (text: string) => void;
  onFinishEditing?: () => void;
}

export const CreativeTextNode: React.FC<CreativeTextNodeProps> = ({
  title,
  emptyLabel,
  editing = false,
  onTextChange,
  onFinishEditing,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const fontSize = Math.min(48, Math.max(10, node.data.fontSize));
  const resolvedTitle = title ?? t('creativeStudio.canvas.nodeKinds.text');
  const resolvedEmptyLabel =
    emptyLabel ?? t('creativeStudio.canvas.nodes.text.empty');
  return (
    <CreativeNodeFrame
      node={node}
      title={resolvedTitle}
      {...sharedFrameProps(props)}
    >
      {editing && !node.locked ? (
        <textarea
          className={styles.textEditor}
          style={{ fontSize, textAlign: node.data.textAlign }}
          value={node.data.text}
          placeholder={resolvedEmptyLabel}
          maxLength={1_000_000}
          autoFocus
          aria-label={t('creativeStudio.canvas.properties.content', {
            defaultValue: '内容',
          })}
          data-node-text-editor
          data-node-text-format={node.data.format}
          onFocus={(event) => {
            if (event.currentTarget.dataset.initialCaretPlaced) return;
            event.currentTarget.dataset.initialCaretPlaced = 'true';
            const end = event.currentTarget.value.length;
            event.currentTarget.setSelectionRange(end, end);
          }}
          onChange={(event) => onTextChange?.(event.currentTarget.value)}
          onBlur={() => onFinishEditing?.()}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (
              event.nativeEvent.isComposing ||
              (event.nativeEvent as KeyboardEvent & { keyCode?: number }).keyCode ===
                229
            ) {
              return;
            }
            if (event.key === 'Escape') {
              event.preventDefault();
              onFinishEditing?.();
              return;
            }
            if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              onFinishEditing?.();
            }
          }}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onDoubleClick={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.stopPropagation()}
          onWheel={(event) => event.stopPropagation()}
        />
      ) : (
        <div
          className={styles.textContent}
          style={{ fontSize, textAlign: node.data.textAlign }}
          data-node-text-format={node.data.format}
        >
          {node.data.text || (
            <span className={styles.emptyText}>{resolvedEmptyLabel}</span>
          )}
        </div>
      )}
    </CreativeNodeFrame>
  );
};

interface CreativeAssetNodeProps<K extends 'image' | 'video' | 'audio' | 'panorama'>
  extends CreativeNodePresentationProps<K> {
  asset?: CreativeNodeAssetPresentation | null;
  title?: string;
  emptyLabel?: string;
}

export type CreativeImageNodeProps = CreativeAssetNodeProps<'image'>;

export const CreativeImageNode: React.FC<CreativeImageNodeProps> = ({
  asset,
  title,
  emptyLabel,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const resolvedTitle = title ?? t('creativeStudio.canvas.nodeKinds.image');
  const resolvedEmptyLabel =
    asset?.deleted ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
      : emptyLabel ?? t('creativeStudio.canvas.nodes.image.empty');
  return (
    <CreativeNodeFrame
      node={node}
      title={resolvedTitle}
      footer={node.data.naturalSize ? `${node.data.naturalSize.width} × ${node.data.naturalSize.height}` : undefined}
      {...sharedFrameProps(props)}
    >
      {resolved ? (
        <CreativeMediaPreview
          kind='image'
          className={styles.imageMedia}
          src={asset?.originalSrc ?? asset?.src}
          posterSrc={asset?.src}
          alt={asset?.alt ?? node.data.alt}
          fit={node.data.fit}
        />
      ) : (
        <EmptyMedia
          icon={<Pic theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={resolvedEmptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export type CreativeVideoNodeProps = CreativeAssetNodeProps<'video'>;

export const CreativeVideoNode: React.FC<CreativeVideoNodeProps> = ({
  asset,
  title,
  emptyLabel,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const resolvedTitle = title ?? t('creativeStudio.canvas.nodeKinds.video');
  const resolvedEmptyLabel =
    asset?.deleted ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
      : emptyLabel ?? t('creativeStudio.canvas.nodes.video.empty');
  const trimLabel = `${formatMilliseconds(node.data.trimStartMs)} – ${
    node.data.trimEndMs == null ? '∞' : formatMilliseconds(node.data.trimEndMs)
  }`;
  return (
    <CreativeNodeFrame
      node={node}
      title={resolvedTitle}
      footer={resolved ? trimLabel : undefined}
      {...sharedFrameProps(props)}
    >
      {resolved && asset ? (
        <CreativeVideoNodeMedia
          key={`${node.id}:${asset.src}`}
          node={node}
          asset={asset}
          title={resolvedTitle}
          selected={props.selected}
          onActivate={nodeCallbacks(node, props).onActivate}
        />
      ) : (
        <EmptyMedia
          icon={<VideoTwo theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={resolvedEmptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export type CreativeAudioNodeProps = CreativeAssetNodeProps<'audio'>;

export const CreativeAudioNode: React.FC<CreativeAudioNodeProps> = ({
  asset,
  title,
  emptyLabel,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const resolvedTitle = title ?? t('creativeStudio.canvas.nodeKinds.audio');
  const resolvedEmptyLabel =
    asset?.deleted ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
      : emptyLabel ?? t('creativeStudio.canvas.nodes.audio.empty');
  const trimLabel = `${formatMilliseconds(node.data.trimStartMs)} – ${
    node.data.trimEndMs == null ? '∞' : formatMilliseconds(node.data.trimEndMs)
  } · ${Math.round(Math.min(1, Math.max(0, node.data.volume)) * 100)}%`;
  return (
    <CreativeNodeFrame
      node={node}
      title={node.data.title || resolvedTitle}
      footer={resolved ? trimLabel : undefined}
      {...sharedFrameProps(props)}
    >
      {resolved ? (
        <div className={styles.audioContent}>
          <Voice theme='outline' size={30} fill='currentColor' strokeWidth={2.5} />
          <audio
            className={styles.audioPlayer}
            src={asset?.src}
            controls
            loop={node.data.loop}
            preload='metadata'
            aria-label={
              asset?.alt ??
              asset?.label ??
              node.data.title ??
              resolvedTitle
            }
            onPointerDown={(event) => event.stopPropagation()}
          />
        </div>
      ) : (
        <EmptyMedia
          icon={<Voice theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={resolvedEmptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export interface CreativePanoramaNodeProps extends CreativeAssetNodeProps<'panorama'> {
  preview?: React.ReactNode;
}

export const CreativePanoramaNode: React.FC<CreativePanoramaNodeProps> = ({
  asset,
  preview,
  title,
  emptyLabel,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const resolvedTitle =
    title ?? t('creativeStudio.canvas.nodeKinds.panorama');
  const resolvedEmptyLabel =
    asset?.deleted ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
      : emptyLabel ?? t('creativeStudio.canvas.nodes.panorama.empty');
  return (
    <CreativeNodeFrame
      node={node}
      title={resolvedTitle}
      footer={t('creativeStudio.canvas.nodes.panorama.orientation', {
        yaw: Math.round(node.data.yaw),
        pitch: Math.round(node.data.pitch),
      })}
      {...sharedFrameProps(props)}
    >
      {preview && !asset?.deleted ? (
        <div className={styles.previewSlot} data-node-preview='panorama'>
          {preview}
        </div>
      ) : resolved ? (
        <CreativeMediaPreview
          kind='image'
          className={styles.imageMedia}
          src={asset?.originalSrc ?? asset?.src}
          posterSrc={asset?.src}
          alt={asset?.alt ?? asset?.label ?? resolvedTitle}
        />
      ) : (
        <EmptyMedia
          icon={<PanoramaHorizontal theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={resolvedEmptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export interface CreativeDirectorNodeProps extends CreativeNodePresentationProps<'director'> {
  title?: string;
  emptyLabel?: string;
  preview?: React.ReactNode;
}

export const CreativeDirectorNode: React.FC<CreativeDirectorNodeProps> = ({
  title,
  emptyLabel,
  preview,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const timeline = Math.max(0, node.data.timelineMs);
  const duration = Math.max(0, node.data.durationMs);
  const resolvedTitle =
    title ?? t('creativeStudio.canvas.nodeKinds.director');
  const resolvedEmptyLabel =
    emptyLabel ?? t('creativeStudio.canvas.nodes.director.empty');
  return (
    <CreativeNodeFrame
      node={node}
      title={resolvedTitle}
      footer={`${formatMilliseconds(timeline)} / ${formatMilliseconds(duration)}`}
      {...sharedFrameProps(props)}
    >
      {preview ? (
        <div className={styles.previewSlot} data-node-preview='director'>
          {preview}
        </div>
      ) : (
        <div className={styles.directorContent}>
          <Camera theme='outline' size={28} fill='currentColor' strokeWidth={2.5} />
          <strong>{node.data.cameraId ?? resolvedEmptyLabel}</strong>
          <progress
            value={Math.min(timeline, duration)}
            max={Math.max(duration, 1)}
            aria-label={t('creativeStudio.canvas.nodes.director.timeline')}
          />
        </div>
      )}
    </CreativeNodeFrame>
  );
};

export interface CreativeGroupNodeProps extends CreativeNodePresentationProps<'group'> {
  titleFallback?: string;
  children?: React.ReactNode;
}

export const CreativeGroupNode: React.FC<CreativeGroupNodeProps> = ({
  titleFallback,
  children,
  ...props
}) => {
  const { t } = useTranslation();
  const { node } = props;
  const resolvedTitleFallback =
    titleFallback ?? t('creativeStudio.canvas.nodes.group.fallback');
  const groupStyle = {
    ...props.style,
    '--creative-node-accent': node.data.color ?? undefined,
  } as React.CSSProperties;
  return (
    <CreativeNodeFrame
      node={node}
      title={node.data.title || resolvedTitleFallback}
      variant='group'
      {...sharedFrameProps({ ...props, style: groupStyle })}
    >
      <div className={styles.groupContent} data-node-group-content>
        {children}
      </div>
    </CreativeNodeFrame>
  );
};

export type CreativeAnyNodeViewProps = CreativeNodePresentationProps<CreativeCanvasNodeKind> & {
  asset?: CreativeNodeAssetPresentation | null;
  panoramaPreview?: React.ReactNode;
  directorPreview?: React.ReactNode;
  groupContent?: React.ReactNode;
  textEditing?: boolean;
  onTextChange?: (text: string) => void;
  onTextEditingComplete?: () => void;
};

/** User-facing views for persisted canvas nodes; task-record configs stay headless. */
export const CreativeNodeView: React.FC<CreativeAnyNodeViewProps> = (props) => {
  const { node } = props;
  switch (node.type) {
    case 'text':
      return (
        <CreativeTextNode
          {...props}
          node={node}
          editing={props.textEditing}
          onTextChange={props.onTextChange}
          onFinishEditing={props.onTextEditingComplete}
        />
      );
    case 'image':
      return <CreativeImageNode {...props} node={node} asset={props.asset} />;
    case 'video':
      return <CreativeVideoNode {...props} node={node} asset={props.asset} />;
    case 'audio':
      return <CreativeAudioNode {...props} node={node} asset={props.asset} />;
    case 'panorama':
      return <CreativePanoramaNode {...props} node={node} asset={props.asset} preview={props.panoramaPreview} />;
    case 'config':
      return null;
    case 'director':
      return <CreativeDirectorNode {...props} node={node} preview={props.directorPreview} />;
    case 'group':
      return <CreativeGroupNode {...props} node={node}>{props.groupContent}</CreativeGroupNode>;
  }
};

export const CREATIVE_NODE_VIEW_KINDS = [
  'image',
  'panorama',
  'text',
  'video',
  'audio',
  'director',
  'group',
] as const satisfies readonly Exclude<CreativeCanvasNodeKind, 'config'>[];
