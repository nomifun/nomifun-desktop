/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Camera,
  Group,
  MovieBoard,
  PanoramaHorizontal,
  Pic,
  SettingConfig,
  Text,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import React from 'react';

import type { CreativeCanvasNodeKind, CreativeGenerationStatus } from '../../domain/schema';
import CreativeNodeFrame from './CreativeNodeFrame';
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
}

export const CreativeTextNode: React.FC<CreativeTextNodeProps> = ({
  title = '文本',
  emptyLabel = '双击编辑文字',
  ...props
}) => {
  const { node } = props;
  const fontSize = Math.min(48, Math.max(10, node.data.fontSize));
  return (
    <CreativeNodeFrame
      node={node}
      icon={<Text theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={node.data.format === 'markdown' ? 'Markdown' : 'Plain text'}
      {...sharedFrameProps(props)}
    >
      <div
        className={styles.textContent}
        style={{ fontSize, textAlign: node.data.textAlign }}
        data-node-text-format={node.data.format}
      >
        {node.data.text || <span className={styles.emptyText}>{emptyLabel}</span>}
      </div>
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
  title = '图片',
  emptyLabel = '空图片节点',
  ...props
}) => {
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  return (
    <CreativeNodeFrame
      node={node}
      icon={<Pic theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={(asset?.label ?? node.data.caption) || undefined}
      footer={node.data.naturalSize ? `${node.data.naturalSize.width} × ${node.data.naturalSize.height}` : undefined}
      {...sharedFrameProps(props)}
    >
      {resolved ? (
        <img
          className={styles.imageMedia}
          src={asset?.src}
          alt={asset?.alt ?? node.data.alt}
          draggable={false}
          style={{ objectFit: node.data.fit }}
        />
      ) : (
        <EmptyMedia
          icon={<Pic theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={emptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export type CreativeVideoNodeProps = CreativeAssetNodeProps<'video'>;

export const CreativeVideoNode: React.FC<CreativeVideoNodeProps> = ({
  asset,
  title = '视频',
  emptyLabel = '空视频节点',
  ...props
}) => {
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const trimLabel = `${formatMilliseconds(node.data.trimStartMs)} – ${
    node.data.trimEndMs == null ? '∞' : formatMilliseconds(node.data.trimEndMs)
  }`;
  return (
    <CreativeNodeFrame
      node={node}
      icon={<VideoTwo theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={asset?.label}
      footer={trimLabel}
      {...sharedFrameProps(props)}
    >
      {resolved ? (
        <video
          className={styles.videoMedia}
          src={asset?.src}
          poster={asset?.posterSrc}
          controls
          muted={node.data.muted}
          loop={node.data.loop}
          autoPlay={node.data.autoplay}
          preload='metadata'
          aria-label={asset?.alt ?? asset?.label ?? title}
          onPointerDown={(event) => event.stopPropagation()}
        />
      ) : (
        <EmptyMedia
          icon={<VideoTwo theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={emptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export type CreativeAudioNodeProps = CreativeAssetNodeProps<'audio'>;

export const CreativeAudioNode: React.FC<CreativeAudioNodeProps> = ({
  asset,
  title = '音频',
  emptyLabel = '空音频节点',
  ...props
}) => {
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  const trimLabel = `${formatMilliseconds(node.data.trimStartMs)} – ${
    node.data.trimEndMs == null ? '∞' : formatMilliseconds(node.data.trimEndMs)
  } · ${Math.round(Math.min(1, Math.max(0, node.data.volume)) * 100)}%`;
  return (
    <CreativeNodeFrame
      node={node}
      icon={<Voice theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={node.data.title || title}
      subtitle={asset?.label}
      footer={trimLabel}
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
            aria-label={asset?.alt ?? asset?.label ?? node.data.title ?? title}
            onPointerDown={(event) => event.stopPropagation()}
          />
        </div>
      ) : (
        <EmptyMedia
          icon={<Voice theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={emptyLabel}
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
  title = '全景图',
  emptyLabel = '空全景图节点',
  ...props
}) => {
  const { node } = props;
  const resolved = Boolean(node.data.assetId && asset?.src);
  return (
    <CreativeNodeFrame
      node={node}
      icon={<PanoramaHorizontal theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={`${Math.round(node.data.fieldOfView)}° FOV`}
      footer={`Yaw ${Math.round(node.data.yaw)}° · Pitch ${Math.round(node.data.pitch)}°`}
      {...sharedFrameProps(props)}
    >
      {preview ? (
        <div className={styles.previewSlot} data-node-preview='panorama'>
          {preview}
        </div>
      ) : resolved ? (
        <img
          className={styles.imageMedia}
          src={asset?.src}
          alt={asset?.alt ?? asset?.label ?? title}
          draggable={false}
        />
      ) : (
        <EmptyMedia
          icon={<PanoramaHorizontal theme='outline' size={25} fill='currentColor' strokeWidth={2.5} />}
          label={emptyLabel}
          assetId={node.data.assetId}
        />
      )}
    </CreativeNodeFrame>
  );
};

export interface CreativeConfigNodeProps extends CreativeNodePresentationProps<'config'> {
  title?: string;
  providerFallback?: string;
  modelFallback?: string;
  promptFallback?: string;
}

export const CreativeConfigNode: React.FC<CreativeConfigNodeProps> = ({
  title = '生成配置',
  providerFallback = '未选择服务商',
  modelFallback = '未选择模型',
  promptFallback = '尚未填写提示词',
  ...props
}) => {
  const { node } = props;
  const canonicalRuntime = {
    status: node.data.status,
    errorMessage: node.data.errorMessage,
  } satisfies { status: CreativeGenerationStatus; errorMessage: string | null };
  return (
    <CreativeNodeFrame
      node={node}
      icon={<SettingConfig theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={`${node.data.task} · ${node.data.capability}`}
      footer={`${Object.keys(node.data.parameters).length} parameters · ${node.data.inputAssetIds.length} inputs`}
      {...sharedFrameProps({ ...props, runtime: props.runtime ?? canonicalRuntime })}
    >
      <div className={styles.configContent}>
        <div className={styles.modelRow}>
          <span>{node.data.providerId ?? providerFallback}</span>
          <strong>{node.data.model ?? modelFallback}</strong>
        </div>
        <p className={styles.prompt}>{node.data.prompt || promptFallback}</p>
        {node.data.negativePrompt ? <p className={styles.negativePrompt}>{node.data.negativePrompt}</p> : null}
      </div>
    </CreativeNodeFrame>
  );
};

export interface CreativeDirectorNodeProps extends CreativeNodePresentationProps<'director'> {
  title?: string;
  emptyLabel?: string;
  preview?: React.ReactNode;
}

export const CreativeDirectorNode: React.FC<CreativeDirectorNodeProps> = ({
  title = '导演台',
  emptyLabel = '尚未建立场景',
  preview,
  ...props
}) => {
  const { node } = props;
  const timeline = Math.max(0, node.data.timelineMs);
  const duration = Math.max(0, node.data.durationMs);
  return (
    <CreativeNodeFrame
      node={node}
      icon={<MovieBoard theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={title}
      subtitle={node.data.sceneId ?? emptyLabel}
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
          <strong>{node.data.cameraId ?? emptyLabel}</strong>
          <progress value={Math.min(timeline, duration)} max={Math.max(duration, 1)} aria-label='Timeline' />
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
  titleFallback = '节点组',
  children,
  ...props
}) => {
  const { node } = props;
  const groupStyle = {
    ...props.style,
    '--creative-node-accent': node.data.color ?? undefined,
  } as React.CSSProperties;
  return (
    <CreativeNodeFrame
      node={node}
      icon={<Group theme='outline' size={15} fill='currentColor' strokeWidth={3} />}
      title={node.data.title || titleFallback}
      subtitle={node.data.collapsed ? 'Collapsed' : undefined}
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
};

/** Canonical discriminated-union dispatcher for the eight persisted node kinds. */
export const CreativeNodeView: React.FC<CreativeAnyNodeViewProps> = (props) => {
  const { node } = props;
  switch (node.type) {
    case 'text':
      return <CreativeTextNode {...props} node={node} />;
    case 'image':
      return <CreativeImageNode {...props} node={node} asset={props.asset} />;
    case 'video':
      return <CreativeVideoNode {...props} node={node} asset={props.asset} />;
    case 'audio':
      return <CreativeAudioNode {...props} node={node} asset={props.asset} />;
    case 'panorama':
      return <CreativePanoramaNode {...props} node={node} asset={props.asset} preview={props.panoramaPreview} />;
    case 'config':
      return <CreativeConfigNode {...props} node={node} />;
    case 'director':
      return <CreativeDirectorNode {...props} node={node} preview={props.directorPreview} />;
    case 'group':
      return <CreativeGroupNode {...props} node={node}>{props.groupContent}</CreativeGroupNode>;
  }
};

// Ensure the canonical union cannot gain a new kind without making this module
// visibly incomplete to consumers and tests.
export const CREATIVE_NODE_VIEW_KINDS = [
  'image',
  'panorama',
  'text',
  'config',
  'video',
  'audio',
  'director',
  'group',
] as const satisfies readonly CreativeCanvasNodeKind[];
