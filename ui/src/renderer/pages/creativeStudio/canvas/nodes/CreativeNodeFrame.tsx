/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Check, Close, Error, Loading, Lock, Unlock } from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';

import type { CreativeCanvasNode, CreativeGenerationStatus } from '../../domain/schema';
import type { CreativeNodePlacement, CreativeNodeRuntimePresentation } from './types';
import styles from './CreativeNodeFrame.module.css';

export interface CreativeNodeStatusLabels {
  idle: string;
  queued: string;
  running: string;
  succeeded: string;
  failed: string;
  canceled: string;
  locked: string;
  lock: string;
  unlock: string;
}

export interface CreativeNodeFrameProps {
  node: CreativeCanvasNode;
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
  children?: React.ReactNode;
  footer?: React.ReactNode;
  selected?: boolean;
  placement?: CreativeNodePlacement;
  runtime?: CreativeNodeRuntimePresentation;
  variant?: 'card' | 'group';
  className?: string;
  style?: React.CSSProperties;
  headerActions?: React.ReactNode;
  inputHandle?: React.ReactNode;
  outputHandle?: React.ReactNode;
  labels?: Partial<CreativeNodeStatusLabels>;
  onActivate?: () => void;
  onOpen?: () => void;
  onToggleLock?: () => void;
  onPointerDown?: React.PointerEventHandler<HTMLElement>;
  onContextMenu?: React.MouseEventHandler<HTMLElement>;
}

const DEFAULT_LABELS: CreativeNodeStatusLabels = {
  idle: '空闲',
  queued: '排队中',
  running: '运行中',
  succeeded: '已完成',
  failed: '失败',
  canceled: '已取消',
  locked: '节点已锁定',
  lock: '锁定节点',
  unlock: '解锁节点',
};

const statusIcon = (status: CreativeGenerationStatus) => {
  switch (status) {
    case 'queued':
    case 'running':
      return <Loading theme='outline' size={13} fill='currentColor' strokeWidth={3} />;
    case 'succeeded':
      return <Check theme='outline' size={13} fill='currentColor' strokeWidth={3} />;
    case 'failed':
      return <Error theme='outline' size={13} fill='currentColor' strokeWidth={3} />;
    case 'canceled':
      return <Close theme='outline' size={13} fill='currentColor' strokeWidth={3} />;
    default:
      return null;
  }
};

const finiteOr = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback);

const CreativeNodeFrame: React.FC<CreativeNodeFrameProps> = ({
  node,
  icon,
  title,
  subtitle,
  children,
  footer,
  selected = false,
  placement = 'world',
  runtime,
  variant = 'card',
  className,
  style,
  headerActions,
  inputHandle,
  outputHandle,
  labels,
  onActivate,
  onOpen,
  onToggleLock,
  onPointerDown,
  onContextMenu,
}) => {
  const status = runtime?.status ?? 'idle';
  const statusLabels = { ...DEFAULT_LABELS, ...labels };
  const progress = runtime?.progress == null ? null : Math.min(100, Math.max(0, runtime.progress));
  const layoutStyle: React.CSSProperties =
    placement === 'world'
      ? {
          position: 'absolute',
          left: finiteOr(node.position.x, 0),
          top: finiteOr(node.position.y, 0),
          width: Math.max(1, finiteOr(node.size.width, 1)),
          height: Math.max(1, finiteOr(node.size.height, 1)),
          zIndex: finiteOr(node.zIndex, 0),
        }
      : { width: '100%', height: '100%' };

  const activate = () => onActivate?.();

  return (
    <article
      className={classNames(styles.frame, variant === 'group' && styles.groupFrame, className)}
      style={{ ...layoutStyle, ...style }}
      tabIndex={onActivate ? 0 : undefined}
      aria-label={title}
      aria-selected={selected}
      data-node-id={node.id}
      data-node-type={node.type}
      data-node-selected={selected || undefined}
      data-node-locked={node.locked || undefined}
      data-node-status={status}
      onClick={activate}
      onDoubleClick={(event) => {
        if (!onOpen) return;
        event.stopPropagation();
        onOpen();
      }}
      onKeyDown={(event) => {
        if (!onActivate || (event.key !== 'Enter' && event.key !== ' ')) return;
        event.preventDefault();
        activate();
      }}
      onPointerDown={onPointerDown}
      onContextMenu={onContextMenu}
    >
      {inputHandle ? <div className={styles.inputHandle}>{inputHandle}</div> : null}
      {outputHandle ? <div className={styles.outputHandle}>{outputHandle}</div> : null}

      <header className={styles.header}>
        <span className={styles.kindIcon} aria-hidden='true'>
          {icon}
        </span>
        <span className={styles.titleBlock}>
          <strong className={styles.title}>{title}</strong>
          {subtitle ? <span className={styles.subtitle}>{subtitle}</span> : null}
        </span>
        {status !== 'idle' ? (
          <span className={styles.status} data-status={status} title={runtime?.label ?? statusLabels[status]}>
            <span className={styles.statusIcon} aria-hidden='true'>
              {statusIcon(status)}
            </span>
            <span>{runtime?.label ?? statusLabels[status]}</span>
          </span>
        ) : null}
        {headerActions ? <div className={styles.actions}>{headerActions}</div> : null}
        {onToggleLock ? (
          <button
            type='button'
            className={styles.lockButton}
            title={node.locked ? statusLabels.unlock : statusLabels.lock}
            aria-label={node.locked ? statusLabels.unlock : statusLabels.lock}
            aria-pressed={node.locked}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              onToggleLock();
            }}
          >
            {node.locked ? (
              <Lock theme='outline' size={14} fill='currentColor' strokeWidth={3} />
            ) : (
              <Unlock theme='outline' size={14} fill='currentColor' strokeWidth={3} />
            )}
          </button>
        ) : node.locked ? (
          <span className={styles.lockedIndicator} title={statusLabels.locked} aria-label={statusLabels.locked}>
            <Lock theme='outline' size={13} fill='currentColor' strokeWidth={3} />
          </span>
        ) : null}
      </header>

      <div className={styles.body}>{children}</div>

      {progress != null && (status === 'queued' || status === 'running') ? (
        <div
          className={styles.progress}
          role='progressbar'
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(progress)}
        >
          <span style={{ width: `${progress}%` }} />
        </div>
      ) : null}

      {runtime?.errorMessage && status === 'failed' ? (
        <div className={styles.errorMessage} role='alert' title={runtime.errorMessage}>
          {runtime.errorMessage}
        </div>
      ) : null}

      {footer ? <footer className={styles.footer}>{footer}</footer> : null}
    </article>
  );
};

export default CreativeNodeFrame;
