/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CloseOne } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import type { CreativeCanvasPromptReferenceOption } from './CreativeCanvasReferencePromptInput';
import styles from './CreativeCanvasImageComposer.module.css';

export interface CreativeCanvasImageComposerReference
  extends CreativeCanvasPromptReferenceOption {
  assetId: string | null;
  connectionId: string | null;
  base: boolean;
}

interface CreativeCanvasReferenceListProps {
  references: readonly CreativeCanvasImageComposerReference[];
  disabled?: boolean;
  onActivate?(nodeId: string): void;
  onDisconnect?(connectionId: string): void;
  onDisconnectMany?(connectionIds: readonly string[]): void;
}

/** Selection is transient; removing references always deletes canonical edges. */
const CreativeCanvasReferenceList: React.FC<CreativeCanvasReferenceListProps> = ({
  references,
  disabled = false,
  onActivate,
  onDisconnect,
  onDisconnectMany,
}) => {
  const { t } = useTranslation();
  const [managing, setManaging] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const manageButton = useRef<HTMLButtonElement>(null);
  const connectionIds = [...new Set(references.flatMap((reference) =>
    !reference.base && reference.connectionId ? [reference.connectionId] : []
  ))];
  const connectionKey = JSON.stringify(connectionIds);
  const selected = connectionIds.filter((id) => selectedIds.includes(id));
  const allSelected = connectionIds.length > 0 && selected.length === connectionIds.length;
  const canManage = Boolean(onDisconnectMany) && connectionIds.length > 0;
  const batchMode = managing && canManage;

  useEffect(() => {
    const available = new Set<string>(JSON.parse(connectionKey));
    setSelectedIds((current) => current.filter((id) => available.has(id)));
    if (available.size === 0) setManaging(false);
  }, [connectionKey]);

  const finishManaging = (): void => {
    setManaging(false);
    setSelectedIds([]);
    manageButton.current?.focus();
  };

  if (references.length === 0) return null;
  return (
    <div
      className={styles.referenceSection}
      data-reference-batch={batchMode || undefined}
      onKeyDown={(event) => {
        if (batchMode && event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          finishManaging();
        }
      }}
    >
      <div className={styles.referenceRow}>
        <div
          className={styles.referenceStrip}
          role='list'
          aria-label={t('creativeStudio.canvas.image.connectedReferences', { defaultValue: '已连接参考' })}
        >
          {references.map((reference) => {
            const removable = !reference.base && Boolean(reference.connectionId);
            const checked = Boolean(reference.connectionId && selected.includes(reference.connectionId));
            return (
              <div
                key={reference.nodeId}
                className={styles.referenceItem}
                role='listitem'
                data-base={reference.base || undefined}
                data-unavailable={Boolean(reference.disabledReason) || undefined}
                data-reference-kind={reference.kind ?? 'image'}
              >
                <button
                  type='button'
                  className={styles.referencePreview}
                  title={batchMode && !removable
                    ? t('creativeStudio.canvas.image.baseReferencePinned')
                    : reference.disabledReason ?? reference.textContent ?? reference.label}
                  aria-label={t(batchMode
                    ? 'creativeStudio.canvas.image.selectReference'
                    : 'creativeStudio.canvas.image.locateReference', {
                    name: reference.label,
                    defaultValue: batchMode ? '选择参考 {{name}}' : '定位参考 {{name}}',
                  })}
                  aria-pressed={batchMode ? checked : undefined}
                  disabled={disabled || (batchMode && !removable)}
                  onClick={() => {
                    if (!batchMode) {
                      onActivate?.(reference.nodeId);
                    } else if (reference.connectionId && removable) {
                      const id = reference.connectionId;
                      setSelectedIds((current) => current.includes(id)
                        ? current.filter((candidate) => candidate !== id)
                        : [...current, id]);
                    }
                  }}
                >
                  {reference.kind === 'text' ? (
                    <span className={styles.referenceText}>{reference.textContent || reference.label}</span>
                  ) : reference.thumbnailUrl || reference.originalUrl ? (
                    <CreativeMediaPreview
                      kind='image'
                      src={reference.originalUrl ?? reference.thumbnailUrl}
                      posterSrc={reference.thumbnailUrl}
                      alt=''
                    />
                  ) : (
                    <span aria-hidden='true'>—</span>
                  )}
                  {reference.kind !== 'text' ? (
                    <strong>{reference.disabledReason ? '!' : reference.ordinal}</strong>
                  ) : null}
                  {batchMode && removable ? (
                    <span className={styles.referenceCheck} aria-hidden='true'>{checked ? '✓' : ''}</span>
                  ) : null}
                </button>
                <span className={styles.referenceName} title={reference.label}>
                  {reference.kind === 'text' ? reference.mentionLabel : reference.label}
                </span>
                {!batchMode && removable && onDisconnect ? (
                  <button
                    type='button'
                    className={styles.referenceRemove}
                    aria-label={t('creativeStudio.canvas.image.disconnectReference', {
                      name: reference.label, defaultValue: '断开参考 {{name}}',
                    })}
                    disabled={disabled}
                    onClick={() => onDisconnect(reference.connectionId!)}
                  >
                    <CloseOne theme='two-tone' size={12} strokeWidth={3} fill={['currentColor', 'var(--color-bg-popup)']} />
                  </button>
                ) : null}
              </div>
            );
          })}
        </div>
        {canManage ? (
          <button
            ref={manageButton}
            type='button'
            className={styles.referenceBatchButton}
            aria-pressed={batchMode}
            disabled={disabled}
            onClick={() => {
              if (batchMode) finishManaging();
              else setManaging(true);
            }}
          >
            {t(batchMode ? 'creativeStudio.canvas.image.cancelBatch' : 'creativeStudio.canvas.image.manageReferences')}
          </button>
        ) : null}
      </div>
      {batchMode ? (
        <div className={styles.referenceBatchActions} role='group' aria-label={t('creativeStudio.canvas.image.manageReferences')}>
          <button
            type='button'
            className={styles.referenceBatchButton}
            aria-pressed={allSelected}
            disabled={disabled}
            onClick={() => setSelectedIds(allSelected ? [] : connectionIds)}
          >
            {t(allSelected ? 'creativeStudio.canvas.image.deselectReferences' : 'creativeStudio.canvas.image.selectAllReferences')}
          </button>
          <span aria-live='polite'>{t('creativeStudio.canvas.image.selectedReferenceCount', { count: selected.length })}</span>
          <button
            type='button'
            className={styles.referenceBatchButton}
            data-danger
            disabled={disabled || selected.length === 0}
            onClick={() => {
              if (disabled || selected.length === 0) return;
              onDisconnectMany?.(selected);
              finishManaging();
            }}
          >
            {t('creativeStudio.canvas.image.disconnectSelectedReferences')}
          </button>
        </div>
      ) : null}
    </div>
  );
};

export default CreativeCanvasReferenceList;
