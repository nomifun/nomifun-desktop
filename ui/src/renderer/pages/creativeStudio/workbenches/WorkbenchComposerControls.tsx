/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BottomBar, CloseSmall, LeftBar, Pic, Plus, VideoTwo } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import CreativeMediaPreview, { type CreativeMediaPreviewProps } from '../assets/components/CreativeMediaPreview';
import styles from './WorkbenchComposer.module.css';

export const WorkbenchComposerHeader: React.FC<{
  kind: 'image' | 'video';
  layout: 'side' | 'bottom';
  onLayoutChange: (layout: 'side' | 'bottom') => void;
}> = ({ kind, layout, onLayoutChange }) => {
  const { t } = useTranslation();
  return (
    <header className={styles.header}>
      <div className={styles.heading}>
        {kind === 'image' ? <Pic size={20} /> : <VideoTwo size={20} />}
        <div>
          <h1>{t(`creativeStudio.${kind}.header.title`, { defaultValue: kind === 'image' ? '生图工作台' : '视频创作台' })}</h1>
          <small>{t(`creativeStudio.${kind}.header.settings`, { defaultValue: '生成设置' })}</small>
        </div>
      </div>
      <div className={styles.layoutSwitch} role='group' aria-label={t(`creativeStudio.${kind}.layout.label`, { defaultValue: '工作台布局' })}>
        <button type='button' aria-pressed={layout === 'side'} onClick={() => onLayoutChange('side')}>
          <LeftBar size={14} />
          <span>{t(`creativeStudio.${kind}.layout.side`, { defaultValue: '侧边' })}</span>
        </button>
        <button type='button' aria-pressed={layout === 'bottom'} onClick={() => onLayoutChange('bottom')}>
          <BottomBar size={14} />
          <span>{t(`creativeStudio.${kind}.layout.bottom`, { defaultValue: '底部' })}</span>
        </button>
      </div>
    </header>
  );
};

export const WorkbenchReferenceCount: React.FC<{ count: number }> = ({ count }) => {
  const { t } = useTranslation();
  return count > 0 ? (
    <span className={styles.referenceCount} data-workbench-reference-count={count}>
      {t('creativeStudio.workbenchComposer.referenceCount', { count, defaultValue: '{{count}} 项' })}
    </span>
  ) : null;
};

export const WorkbenchAddReference: React.FC<{
  label: string;
  onClick: () => void;
  disabled?: boolean;
}> = ({ label, onClick, disabled }) => (
  <button type='button' className={styles.addReference} data-workbench-add-reference onClick={onClick} disabled={disabled}>
    <Plus size={18} />
    <span>{label}</span>
  </button>
);

export const WorkbenchReferenceCard: React.FC<{
  kind: CreativeMediaPreviewProps['kind'];
  src?: string | null;
  posterSrc?: string | null;
  name: string;
  removeLabel: string;
  onRemove: () => void;
  orderControls?: React.ReactNode;
}> = ({ kind, src, posterSrc, name, removeLabel, onRemove, orderControls }) => (
  <article className={styles.referenceCard} data-reference-kind={kind}>
    <div className={styles.referencePreview}>
      <CreativeMediaPreview kind={kind} src={src} posterSrc={posterSrc} alt={name} className={styles.referenceMedia} />
    </div>
    <span className={styles.referenceName} title={name}>
      {name}
    </span>
    {orderControls}
    <button
      type='button'
      className={styles.referenceRemove}
      aria-label={removeLabel}
      onClick={(event) => {
        const strip = event.currentTarget.closest('article')?.parentElement;
        if (strip?.querySelectorAll('[data-reference-kind]').length === 1) {
          strip.querySelector<HTMLButtonElement>('[data-workbench-add-reference]')?.focus();
        }
        onRemove();
      }}
    >
      <CloseSmall size={14} />
    </button>
  </article>
);
