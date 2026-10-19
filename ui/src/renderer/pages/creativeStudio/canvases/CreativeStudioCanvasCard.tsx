/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input, Spin } from '@arco-design/web-react';
import { Check, Close, Delete, Download, EditTwo, LayoutThree } from '@icon-park/react';
import React, { useEffect, useRef } from 'react';

import type { CreativeCanvasSummary } from '../domain';
import { formatCanvasTimestamp } from './canvasList';
import type { CreativeStudioCanvasesCopy } from './copy';
import CanvasActionsMenu from './CanvasActionsMenu';
import styles from './CreativeStudioCanvasesPage.module.css';

interface CreativeStudioCanvasCardProps {
  canvas: CreativeCanvasSummary;
  copy: CreativeStudioCanvasesCopy;
  language?: string;
  selected: boolean;
  selectionMode?: boolean;
  editing: boolean;
  editingTitle: string;
  disabled?: boolean;
  opening?: boolean;
  exportDisabled?: boolean;
  archiveUnavailableMessage?: string;
  onOpen: (canvas: CreativeCanvasSummary) => void;
  onPrefetch?: (canvas: CreativeCanvasSummary) => void;
  onToggleSelected: (canvas: CreativeCanvasSummary, selected: boolean) => void;
  onStartRename: (canvas: CreativeCanvasSummary) => void;
  onEditingTitleChange: (title: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onExport: (canvas: CreativeCanvasSummary) => void;
  onDelete: (canvas: CreativeCanvasSummary) => void;
}

const CreativeStudioCanvasCard: React.FC<CreativeStudioCanvasCardProps> = ({
  canvas, copy, language, selected, selectionMode = false,
  editing, editingTitle, disabled = false, opening = false,
  exportDisabled = false, archiveUnavailableMessage,
  onOpen, onPrefetch, onToggleSelected, onStartRename,
  onEditingTitleChange, onSaveRename, onCancelRename, onExport, onDelete,
}) => {
  const prefetchTimerRef = useRef<number | null>(null);
  const empty = canvas.nodeCount === 0 && canvas.connectionCount === 0;
  const moreLabel = `${copy.moreActions}: ${canvas.title}`;

  const cancelPrefetch = () => {
    if (prefetchTimerRef.current === null) return;
    window.clearTimeout(prefetchTimerRef.current);
    prefetchTimerRef.current = null;
  };
  const prefetchNow = () => {
    cancelPrefetch();
    if (!disabled && !editing && !selectionMode) onPrefetch?.(canvas);
  };
  const schedulePrefetch = () => {
    if (!onPrefetch || disabled || editing || selectionMode || prefetchTimerRef.current !== null) return;
    prefetchTimerRef.current = window.setTimeout(() => {
      prefetchTimerRef.current = null;
      onPrefetch(canvas);
    }, 120);
  };
  useEffect(() => cancelPrefetch, []);
  useEffect(() => {
    cancelPrefetch();
  }, [disabled, editing, selectionMode]);

  const activate = () => {
    if (editing || disabled) return;
    if (selectionMode) onToggleSelected(canvas, !selected);
    else onOpen(canvas);
  };
  const saveDisabled = disabled || editingTitle.trim().length === 0;

  return (
    <article
      className={styles.card}
      aria-busy={opening}
      data-canvas-id={canvas.canvasId}
      data-canvas-empty={empty ? 'true' : 'false'}
      data-canvas-selected={selected ? 'true' : 'false'}
      data-canvas-opening={opening ? 'true' : 'false'}
      onMouseEnter={schedulePrefetch}
      onMouseLeave={cancelPrefetch}
    >
      <button
        type='button'
        className={styles.cardOpen}
        title={canvas.title}
        aria-label={selectionMode ? copy.selectCanvas(canvas.title) : `${copy.openCanvas}: ${canvas.title}`}
        aria-pressed={selectionMode ? selected : undefined}
        disabled={disabled || editing}
        onFocus={prefetchNow}
        onPointerDown={prefetchNow}
        onClick={activate}
      />

      {selectionMode && (
        <Checkbox
          className={styles.cardCheckbox}
          checked={selected}
          disabled={disabled}
          aria-label={copy.selectCanvas(canvas.title)}
          onChange={(checked) => onToggleSelected(canvas, checked)}
        />
      )}

      <div className={styles.cardArtwork} aria-hidden='true'>
        <span className={styles.cardArtworkMark}>
          {empty ? <span className={styles.cardBlankMark} /> : (
            <LayoutThree theme='outline' size={20} fill='currentColor' />
          )}
        </span>
      </div>

      <div className={styles.cardIdentity}>
        {editing ? (
          <div className={styles.cardRename}>
            <Input
              autoFocus
              value={editingTitle}
              maxLength={80}
              disabled={disabled}
              aria-label={copy.renamePlaceholder}
              placeholder={copy.renamePlaceholder}
              onChange={onEditingTitleChange}
              onKeyDown={(event) => {
                if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
                if (event.key === 'Enter') {
                  event.preventDefault();
                  if (!saveDisabled) onSaveRename();
                } else if (event.key === 'Escape') {
                  event.preventDefault();
                  onCancelRename();
                }
              }}
            />
            <Button type='text' size='small' icon={<Check theme='outline' size={14} fill='currentColor' />}
              aria-label={copy.saveRename} title={copy.saveRename} disabled={saveDisabled} onClick={onSaveRename} />
            <Button type='text' size='small' icon={<Close theme='outline' size={14} fill='currentColor' />}
              aria-label={copy.cancelRename} title={copy.cancelRename} disabled={disabled} onClick={onCancelRename} />
          </div>
        ) : (
          <h2 className={styles.cardTitle} title={canvas.title}>{canvas.title}</h2>
        )}
        <p className={styles.cardStats} title={copy.canvasStats(canvas.nodeCount, canvas.connectionCount)}>
          {empty ? copy.emptyCanvas : copy.canvasStats(canvas.nodeCount, canvas.connectionCount)}
        </p>
        <p className={styles.cardTimestamp}>
          {copy.updatedAt(formatCanvasTimestamp(canvas.updatedAt, language))}
        </p>
      </div>

      {!editing && (
        <CanvasActionsMenu
          key={selectionMode ? 'selection' : 'normal'}
          label={moreLabel}
          disabled={disabled}
          triggerClassName={styles.cardMoreAction}
          actions={[
            { key: 'rename', label: copy.renameCanvas, icon: <EditTwo theme='outline' size={15} fill='currentColor' />, onSelect: () => onStartRename(canvas) },
            { key: 'export', label: copy.exportCanvas, icon: <Download theme='outline' size={15} fill='currentColor' />, onSelect: () => onExport(canvas), disabled: exportDisabled, disabledReason: archiveUnavailableMessage },
            { key: 'delete', label: copy.deleteCanvas, icon: <Delete theme='outline' size={15} fill='currentColor' />, onSelect: () => onDelete(canvas), danger: true },
          ]}
        />
      )}
      {opening && (
        <div className={styles.cardOpening} role='status' aria-live='polite'>
          <Spin dot size={16} />
          <span>{copy.openingCanvas(canvas.title)}</span>
        </div>
      )}
    </article>
  );
};

export default CreativeStudioCanvasCard;
