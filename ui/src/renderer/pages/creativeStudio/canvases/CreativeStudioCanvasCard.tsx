/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input } from '@arco-design/web-react';
import { Check, Close, Delete, Download, EditTwo } from '@icon-park/react';
import React from 'react';

import type { CreativeCanvasSummary } from '../domain';
import { formatCanvasTimestamp } from './canvasList';
import type { CreativeStudioCanvasesCopy } from './copy';
import styles from './CreativeStudioCanvasesPage.module.css';

interface CreativeStudioCanvasCardProps {
  canvas: CreativeCanvasSummary;
  copy: CreativeStudioCanvasesCopy;
  language?: string;
  selected: boolean;
  editing: boolean;
  editingTitle: string;
  disabled?: boolean;
  exportDisabled?: boolean;
  archiveUnavailableMessage?: string;
  onOpen: (canvas: CreativeCanvasSummary) => void;
  onToggleSelected: (canvas: CreativeCanvasSummary, selected: boolean) => void;
  onStartRename: (canvas: CreativeCanvasSummary) => void;
  onEditingTitleChange: (title: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onExport: (canvas: CreativeCanvasSummary) => void;
  onDelete: (canvas: CreativeCanvasSummary) => void;
}

const CreativeStudioCanvasCard: React.FC<CreativeStudioCanvasCardProps> = ({
  canvas,
  copy,
  language,
  selected,
  editing,
  editingTitle,
  disabled = false,
  exportDisabled = false,
  archiveUnavailableMessage,
  onOpen,
  onToggleSelected,
  onStartRename,
  onEditingTitleChange,
  onSaveRename,
  onCancelRename,
  onExport,
  onDelete,
}) => {
  const open = () => {
    if (!editing && !disabled) onOpen(canvas);
  };
  const saveDisabled = disabled || editingTitle.trim().length === 0;

  return (
    <article
      className={styles.card}
      role='button'
      tabIndex={disabled || editing ? -1 : 0}
      aria-label={`${copy.openCanvas}: ${canvas.title}`}
      data-canvas-id={canvas.canvasId}
      data-canvas-selected={selected ? 'true' : 'false'}
      onClick={open}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          open();
        }
      }}
    >
      <div className={styles.cardTop}>
        <Checkbox
          className={styles.cardCheckbox}
          checked={selected}
          disabled={disabled}
          aria-label={copy.selectCanvas(canvas.title)}
          onClick={(event) => event.stopPropagation()}
          onChange={(checked) => onToggleSelected(canvas, checked)}
        />

        <div className={styles.cardIdentity}>
          {editing ? (
            <Input
              autoFocus
              value={editingTitle}
              maxLength={80}
              aria-label={copy.renamePlaceholder}
              placeholder={copy.renamePlaceholder}
              onClick={(event) => event.stopPropagation()}
              onChange={onEditingTitleChange}
              onPressEnter={() => {
                if (!saveDisabled) onSaveRename();
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') onCancelRename();
              }}
            />
          ) : (
            <>
              <h2 className={styles.cardTitle}>{canvas.title}</h2>
              <p className={styles.cardStats}>
                {copy.canvasStats(canvas.nodeCount, canvas.connectionCount)}
              </p>
            </>
          )}
        </div>
      </div>

      <div className={styles.cardFooter}>
        <p className={styles.cardTimestamp}>
          {copy.updatedAt(formatCanvasTimestamp(canvas.updatedAt, language))}
        </p>
        <div
          className={styles.cardActions}
          onClick={(event) => event.stopPropagation()}
        >
          {editing ? (
            <>
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Check theme='outline' size={16} fill='currentColor' />}
                aria-label={copy.saveRename}
                title={copy.saveRename}
                disabled={saveDisabled}
                onClick={onSaveRename}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Close theme='outline' size={16} fill='currentColor' />}
                aria-label={copy.cancelRename}
                title={copy.cancelRename}
                disabled={disabled}
                onClick={onCancelRename}
              />
            </>
          ) : (
            <>
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Download theme='outline' size={16} fill='currentColor' />}
                aria-label={`${copy.exportCanvas}: ${canvas.title}`}
                title={
                  exportDisabled
                    ? archiveUnavailableMessage
                    : copy.exportCanvas
                }
                disabled={disabled || exportDisabled}
                onClick={() => onExport(canvas)}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<EditTwo theme='outline' size={16} fill='currentColor' />}
                aria-label={`${copy.renameCanvas}: ${canvas.title}`}
                title={copy.renameCanvas}
                disabled={disabled}
                onClick={() => onStartRename(canvas)}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Delete theme='outline' size={16} fill='currentColor' />}
                aria-label={`${copy.deleteCanvas}: ${canvas.title}`}
                title={copy.deleteCanvas}
                disabled={disabled}
                onClick={() => onDelete(canvas)}
              />
            </>
          )}
        </div>
      </div>
    </article>
  );
};

export default CreativeStudioCanvasCard;
