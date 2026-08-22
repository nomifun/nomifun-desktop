/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Attention, Camera, Magic, Plus, Right, Timeline } from '@icon-park/react';
import React from 'react';

import type { CreativeCanvasNode } from '../../domain';
import type { CanvasState } from '../core';

import styles from './CreativeCanvasTimelinePanel.module.css';

const iconProps = {
  theme: 'outline' as const,
  fill: 'currentColor',
  strokeWidth: 2.4,
};

export type CreativeCanvasDirectorNode = Extract<CreativeCanvasNode, { type: 'director' }>;

export interface CreativeCanvasTimelinePanelProps {
  state: Pick<CanvasState, 'document'>;
  disabled?: boolean;
  onSelectNode(nodeId: string): void;
  onAddDirector(): void;
  onOpenDirector(nodeId: string): void;
}

export interface CreativeDirectorTimelineProjection {
  currentMs: number;
  durationMs: number;
  progress: number;
}

const finiteNonNegative = (value: number): number => (Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0);

export function creativeCanvasDirectorNodes(state: Pick<CanvasState, 'document'>): CreativeCanvasDirectorNode[] {
  return state.document.nodes.filter((node): node is CreativeCanvasDirectorNode => node.type === 'director');
}

export function projectCreativeDirectorTimeline(node: CreativeCanvasDirectorNode): CreativeDirectorTimelineProjection {
  const durationMs = finiteNonNegative(node.data.durationMs);
  const rawCurrentMs = finiteNonNegative(node.data.timelineMs);
  const currentMs = durationMs > 0 ? Math.min(rawCurrentMs, durationMs) : 0;
  return {
    currentMs,
    durationMs,
    progress: durationMs > 0 ? currentMs / durationMs : 0,
  };
}

export function formatCreativeDirectorTime(milliseconds: number): string {
  const total = finiteNonNegative(milliseconds);
  const minutes = Math.floor(total / 60_000);
  const seconds = Math.floor((total % 60_000) / 1_000);
  const millis = total % 1_000;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}.${String(millis).padStart(3, '0')}`;
}

const EmptyDirectorTimeline: React.FC<Pick<CreativeCanvasTimelinePanelProps, 'disabled' | 'onAddDirector'>> = ({
  disabled,
  onAddDirector,
}) => (
  <section
    className={styles.emptyState}
    data-canvas-product-panel='director-timeline'
    data-director-timeline-state='empty'
    role='status'
  >
    <span className={styles.emptyIcon} aria-hidden='true'>
      <Magic {...iconProps} size={23} />
    </span>
    <div className={styles.emptyCopy}>
      <h2>还没有导演场景</h2>
      <p>添加唯一的导演节点后，可从这里查看真实场景时间并进入 3D 导演台。</p>
    </div>
    <button type='button' className={styles.primaryAction} disabled={disabled} onClick={onAddDirector}>
      <Plus {...iconProps} size={15} />
      添加导演节点
    </button>
  </section>
);

const ConflictingDirectorTimeline: React.FC<
  Pick<CreativeCanvasTimelinePanelProps, 'disabled' | 'onSelectNode'> & {
    nodes: readonly CreativeCanvasDirectorNode[];
  }
> = ({ disabled, nodes, onSelectNode }) => (
  <section
    className={styles.conflictState}
    data-canvas-product-panel='director-timeline'
    data-director-timeline-state='conflict'
    role='alert'
  >
    <div className={styles.conflictCopy}>
      <span className={styles.conflictIcon} aria-hidden='true'>
        <Attention {...iconProps} size={20} />
      </span>
      <div>
        <h2>检测到多个导演节点</h2>
        <p>一个项目只能绑定一个导演场景。请检查并删除多余节点后再打开导演台。</p>
      </div>
    </div>
    <div className={styles.conflictNodes} aria-label='冲突的导演节点'>
      {nodes.map((node, index) => (
        <button type='button' key={node.id} disabled={disabled} title={node.id} onClick={() => onSelectNode(node.id)}>
          <Camera {...iconProps} size={14} />
          <span>导演节点 {index + 1}</span>
          <small>{node.id}</small>
        </button>
      ))}
    </div>
  </section>
);

const ReadyDirectorTimeline: React.FC<
  Pick<CreativeCanvasTimelinePanelProps, 'disabled' | 'onSelectNode' | 'onOpenDirector'> & {
    node: CreativeCanvasDirectorNode;
  }
> = ({ disabled, node, onOpenDirector, onSelectNode }) => {
  const timeline = projectCreativeDirectorTimeline(node);
  const sceneLabel = node.data.sceneId ? '场景已连接' : '场景等待初始化';
  const sceneDetail = node.data.sceneId ? node.data.sceneId : '进入导演台并完成首次编辑后，将建立真实场景资产。';

  return (
    <section
      className={styles.readyState}
      data-canvas-product-panel='director-timeline'
      data-director-timeline-state='ready'
      data-director-node-id={node.id}
    >
      <div className={styles.directorIdentity}>
        <button
          type='button'
          className={styles.identityButton}
          disabled={disabled}
          onClick={() => onSelectNode(node.id)}
        >
          <span className={styles.identityIcon} aria-hidden='true'>
            <Magic {...iconProps} size={20} />
          </span>
          <span className={styles.identityCopy}>
            <small>导演场景</small>
            <strong>{sceneLabel}</strong>
            <span title={sceneDetail}>{sceneDetail}</span>
          </span>
        </button>
      </div>

      <div className={styles.timelineProjection}>
        <div className={styles.timelineMeta}>
          <span className={styles.timelineLabel}>
            <Timeline {...iconProps} size={15} />
            只读时间投影
          </span>
          <time>
            {formatCreativeDirectorTime(timeline.currentMs)}
            <i>/</i>
            {formatCreativeDirectorTime(timeline.durationMs)}
          </time>
        </div>
        <progress value={timeline.currentMs} max={Math.max(timeline.durationMs, 1)} aria-label='导演时间线进度' />
        <div className={styles.cameraRow}>
          <Camera {...iconProps} size={14} />
          <span>当前机位</span>
          <strong title={node.data.cameraId ?? undefined}>{node.data.cameraId ?? '尚未选择'}</strong>
        </div>
      </div>

      <div className={styles.timelineActions}>
        <p>轨道、关键帧与播放控制由导演台维护；画布仅显示已保存的 canonical 投影。</p>
        <button type='button' className={styles.openAction} disabled={disabled} onClick={() => onOpenDirector(node.id)}>
          打开 3D 导演台
          <Right {...iconProps} size={14} />
        </button>
      </div>
    </section>
  );
};

const CreativeCanvasTimelinePanel: React.FC<CreativeCanvasTimelinePanelProps> = (props) => {
  const nodes = creativeCanvasDirectorNodes(props.state);
  if (nodes.length === 0) {
    return <EmptyDirectorTimeline disabled={props.disabled} onAddDirector={props.onAddDirector} />;
  }
  if (nodes.length > 1) {
    return <ConflictingDirectorTimeline disabled={props.disabled} nodes={nodes} onSelectNode={props.onSelectNode} />;
  }
  return (
    <ReadyDirectorTimeline
      disabled={props.disabled}
      node={nodes[0]}
      onSelectNode={props.onSelectNode}
      onOpenDirector={props.onOpenDirector}
    />
  );
};

export default CreativeCanvasTimelinePanel;
