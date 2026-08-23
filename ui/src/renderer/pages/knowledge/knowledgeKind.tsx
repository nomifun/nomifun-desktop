/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared knowledge-base "kind" presentation: badge/icon theming per kind
 * (local / web / blank) plus the rounded-square kind icon. Used by both
 * `KnowledgeCard` (grid item) and `KnowledgeDetailPage` (header).
 *
 * The two surfaces intentionally differ in badge text color: the card uses
 * per-kind accent colors while the detail header stays neutral — pick via
 * `textVariant`.
 */
import type { TFunction } from 'i18next';
import { Earth, EditTwo, FolderOpen } from '@icon-park/react';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';

export type KindConfig = {
  label: string;
  /** UnoCSS bg class (translucent) */
  bgClass: string;
  /** UnoCSS text class */
  textClass: string;
  /** UnoCSS border class */
  borderClass: string;
  /** Icon bg/border CSS vars for the round icon container */
  iconBg: string;
  iconBorder: string;
  iconColor: string;
};

export type KindTextVariant = 'accent' | 'neutral';

/**
 * Per-kind badge + icon styling. Uses theme semantic colors:
 * - blank = neutral/gray (fill-2 / text-2)
 * - local = primary (blue)
 * - web = success (green)
 */
export function getKindConfig(
  kind: IKnowledgeBase['kind'],
  t: TFunction,
  textVariant: KindTextVariant = 'accent'
): KindConfig {
  switch (kind) {
    case 'local':
      return {
        label: t('knowledge.card.kindLocal', { defaultValue: '本地文件夹' }),
        bgClass: 'bg-[rgba(var(--primary-6),0.1)]',
        textClass: textVariant === 'accent' ? 'text-primary-5' : 'text-[var(--color-text-1)]',
        borderClass: 'border-[rgba(var(--primary-6),0.3)]',
        iconBg: 'rgba(var(--primary-6),0.1)',
        iconBorder: 'rgba(var(--primary-6),0.3)',
        iconColor: 'rgb(var(--primary-5))',
      };
    case 'web':
      return {
        label: t('knowledge.card.kindWeb', { defaultValue: '网页' }),
        bgClass: 'bg-[rgba(var(--success-6),0.1)]',
        textClass: textVariant === 'accent' ? 'text-success-5' : 'text-[var(--color-text-1)]',
        borderClass: 'border-[rgba(var(--success-6),0.3)]',
        iconBg: 'rgba(var(--success-6),0.1)',
        iconBorder: 'rgba(var(--success-6),0.3)',
        iconColor: 'rgb(var(--success-5))',
      };
    case 'blank':
    default:
      return {
        label: t('knowledge.card.kindBlank', { defaultValue: '空白' }),
        bgClass: 'bg-fill-2',
        textClass: 'text-[var(--color-text-2)]',
        borderClass: 'border-[var(--color-border-2)]',
        iconBg: 'var(--color-fill-2)',
        iconBorder: 'var(--color-border-2)',
        iconColor: 'var(--color-text-2)',
      };
  }
}

/** Kind icon in a rounded square container (card: 20/42px, detail header: 22/52px). */
export function KindIcon({
  kind,
  config,
  size = 20,
  containerClass = 'w-42px h-42px rounded-12px',
}: {
  kind: IKnowledgeBase['kind'];
  config: KindConfig;
  /** Icon glyph size in px. */
  size?: number;
  /** UnoCSS classes controlling the container dimensions and radius. */
  containerClass?: string;
}) {
  const iconProps = { theme: 'outline' as const, size, strokeWidth: 3 };
  return (
    <div
      className={`${containerClass} flex-none grid place-items-center border border-solid`}
      style={{
        background: config.iconBg,
        borderColor: config.iconBorder,
        color: config.iconColor,
      }}
    >
      {kind === 'local' && <FolderOpen {...iconProps} />}
      {kind === 'web' && <Earth {...iconProps} />}
      {kind === 'blank' && <EditTwo {...iconProps} />}
    </div>
  );
}
