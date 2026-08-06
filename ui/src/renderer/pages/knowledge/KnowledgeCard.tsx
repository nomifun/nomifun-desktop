/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeCard — A grid item for the knowledge base list.
 * Mirrors PresetCard visual language (rounded-16px bordered surface, soft hover)
 * with knowledge-specific additions: kind icon + badge, status tags, user tag chips,
 * meta row, and hover-revealed actions.
 *
 * Theme variables only; `<div onClick>` for clickables (no <button>).
 */
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Tooltip } from '@arco-design/web-react';
import { Delete, EditTwo, LinkOne } from '@icon-park/react';
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { formatSize } from './useKnowledge';
import { getKindConfig, KindIcon } from './knowledgeKind';

// ─── Props ────────────────────────────────────────────────────────────────────

export interface KnowledgeCardProps {
  base: IKnowledgeBase;
  /** Map of tag key → IKnowledgeTag, for resolving base.tags to label + color. */
  tagMap?: Record<string, IKnowledgeTag>;
  onOpen?: (base: IKnowledgeBase) => void;
  onEdit?: (base: IKnowledgeBase) => void;
  onDelete?: (base: IKnowledgeBase, e: React.MouseEvent) => void;
}

// ─── Sub-components ───────────────────────────────────────────────────────────

/** Source-mode status badges (live/snapshot). */
function StatusBadges({
  base,
  t,
}: {
  base: IKnowledgeBase;
  t: TFunction;
}) {
  const badges: React.ReactNode[] = [];

  if (!base.root_exists) {
    badges.push(
      <span
        key='root-missing'
        className='knowledge-card-root-missing inline-flex items-center rounded-6px px-8px py-2px text-10px font-600 border border-solid border-[rgba(var(--danger-6),0.35)] text-danger-6 bg-[rgba(var(--danger-6),0.08)]'
      >
        {t('knowledge.card.rootMissing', { defaultValue: '目录不可用' })}
      </span>
    );
  }

  if (base.source) {
    if (base.source.mode === 'live') {
      badges.push(
        <span
          key='live'
          className='inline-flex items-center rounded-6px px-8px py-2px text-10px font-600 border border-solid border-[rgba(var(--success-6),0.4)] text-success-5 bg-transparent'
        >
          {t('knowledge.card.modeLive', { defaultValue: '实时' })}
        </span>
      );
    } else if (base.source.mode === 'snapshot') {
      badges.push(
        <span
          key='snapshot'
          className='inline-flex items-center rounded-6px px-8px py-2px text-10px font-600 border border-solid border-[var(--color-border-2)] text-[var(--color-text-2)] bg-fill-2'
        >
          {t('knowledge.card.modeSnapshot', { defaultValue: '快照' })}
        </span>
      );
    }
  }

  return badges.length > 0 ? <>{badges}</> : null;
}

const MAX_VISIBLE_TAGS = 5;

/** User tag chips row with colored dots. */
function TagChips({
  tags,
  tagMap,
}: {
  tags: string[];
  tagMap?: Record<string, IKnowledgeTag>;
}) {
  if (!tags.length || !tagMap) return null;

  const resolved = tags
    .map((key) => tagMap[key])
    .filter((t): t is IKnowledgeTag => Boolean(t));

  if (!resolved.length) return null;

  const visibleTags = resolved.slice(0, MAX_VISIBLE_TAGS);
  const overflowCount = resolved.length - visibleTags.length;
  const tooltipContent = (
    <div className='max-w-280px whitespace-normal break-words text-12px leading-18px'>
      {resolved.map((tag) => tag.label).join(' · ')}
    </div>
  );

  return (
    <Tooltip content={tooltipContent} position='top'>
      <div className='knowledge-card-tags flex flex-wrap items-center gap-5px'>
        {visibleTags.map((tag) => (
          <div
            key={tag.key}
            className='inline-flex items-center gap-5px rounded-6px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-2)] px-7px py-1px text-11px leading-16px text-[var(--color-text-2)]'
          >
            {tag.color && (
              <i
                className='h-6px w-6px flex-none rounded-full'
                style={{ background: tag.color }}
              />
            )}
            {tag.label}
          </div>
        ))}
        {overflowCount > 0 && (
          <div className='inline-flex items-center rounded-6px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-2)] px-7px py-1px text-11px font-600 leading-16px text-[var(--color-text-2)]'>
            +{overflowCount}
          </div>
        )}
      </div>
    </Tooltip>
  );
}

/** Relative time format (simple). */
function formatRelativeTime(epochMs: number, t: TFunction): string {
  const now = Date.now();
  const diff = now - epochMs;
  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (seconds < 60) return t('knowledge.card.timeJustNow', { defaultValue: '刚刚' });
  if (minutes < 60) return t('knowledge.card.timeMinutesAgo', { count: minutes, defaultValue: '{{count}} 分钟前' });
  if (hours < 24) return t('knowledge.card.timeHoursAgo', { count: hours, defaultValue: '{{count}} 小时前' });
  if (days === 1) return t('knowledge.card.timeYesterday', { defaultValue: '昨天' });
  if (days < 7) return t('knowledge.card.timeDaysAgo', { count: days, defaultValue: '{{count}} 天前' });
  return t('knowledge.card.timeWeeksAgo', { defaultValue: '上周' });
}

// ─── Main Component ───────────────────────────────────────────────────────────

export const KnowledgeCard: React.FC<KnowledgeCardProps> = ({
  base,
  tagMap,
  onOpen,
  onEdit,
  onDelete,
}) => {
  const { t } = useTranslation();
  const kindConfig = getKindConfig(base.kind, t);
  const metaItems = [
    base.file_count > 0 ? t('knowledge.card.fileCount', { count: base.file_count, defaultValue: '{{count}} 篇' }) : null,
    base.total_size > 0 ? formatSize(base.total_size) : null,
    formatRelativeTime(base.updated_at, t),
  ].filter((item): item is string => Boolean(item));

  return (
    <div
      className={[
        'group relative flex flex-col gap-8px rounded-16px border border-solid',
        'border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-18px pt-18px pb-8px box-border cursor-pointer',
        'min-h-188px',
        'transition-all duration-160',
        'hover:border-[var(--color-border-3)] hover:shadow-[0_14px_38px_rgba(0,0,0,0.15)] hover:-translate-y-2px',
      ].join(' ')}
      onClick={() => onOpen?.(base)}
    >
      {/* Header: icon + name + badges */}
      <div className='flex items-center gap-12px'>
        <KindIcon kind={base.kind} config={kindConfig} />
        <div className='min-w-0 flex-1'>
          <div className='text-15px font-700 leading-[1.3] text-[var(--color-text-1)] truncate'>
            {base.name}
          </div>
          <div className='flex flex-wrap gap-6px mt-4px'>
            {/* Kind badge */}
            <span
              className={[
                'inline-flex items-center rounded-6px px-8px py-2px text-10px font-600 border border-solid',
                kindConfig.bgClass,
                kindConfig.textClass,
                kindConfig.borderClass,
              ].join(' ')}
            >
              {kindConfig.label}
            </span>
            {/* Status badges */}
            <StatusBadges base={base} t={t} />
          </div>
        </div>
      </div>

      {/* Description (2-line clamp) */}
      <div
        className='max-h-40px min-h-0 flex-1 overflow-hidden break-words text-13px leading-20px text-[var(--color-text-2)]'
        style={{
          display: '-webkit-box',
          WebkitLineClamp: 2,
          WebkitBoxOrient: 'vertical',
          overflow: 'hidden',
        }}
      >
        {base.description || t('knowledge.card.noDescription', { defaultValue: '暂无描述' })}
      </div>

      {/* User tags row */}
      <TagChips tags={base.tags} tagMap={tagMap} />

      <div className='knowledge-card-footer mt-auto flex min-h-26px items-center gap-10px'>
        <div className='knowledge-card-meta flex min-w-0 flex-wrap items-center gap-7px text-12px leading-16px text-[var(--color-text-3)]'>
          {metaItems.map((item, index) => (
            <React.Fragment key={`${item}-${index}`}>
              {index > 0 && <i className='h-3px w-3px rounded-full bg-[var(--color-fill-4)]' aria-hidden='true' />}
              <span className='whitespace-nowrap'>{item}</span>
            </React.Fragment>
          ))}
        </div>

        <div
          className='knowledge-card-actions pointer-events-none ml-auto flex shrink-0 gap-6px opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100'
          onClick={(e) => e.stopPropagation()}
        >
          <div
            onClick={() => onOpen?.(base)}
            className={[
              'grid h-26px w-26px place-items-center rounded-7px',
              'border border-solid border-transparent',
              'bg-transparent text-[var(--color-text-3)] cursor-pointer',
              'hover:border-[var(--color-border-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
              'transition-colors',
            ].join(' ')}
            title={t('knowledge.card.actionOpen', { defaultValue: '打开' })}
          >
            <LinkOne theme='outline' size={13} strokeWidth={3} />
          </div>
          <div
            onClick={() => onEdit?.(base)}
            className={[
              'grid h-26px w-26px place-items-center rounded-7px',
              'border border-solid border-transparent',
              'bg-transparent text-[var(--color-text-3)] cursor-pointer',
              'hover:border-[var(--color-border-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
              'transition-colors',
            ].join(' ')}
            title={t('knowledge.card.actionEdit', { defaultValue: '编辑' })}
          >
            <EditTwo theme='outline' size={13} strokeWidth={3} />
          </div>
          <div
            onClick={(e) => onDelete?.(base, e)}
            className={[
              'grid h-26px w-26px place-items-center rounded-7px',
              'border border-solid border-transparent',
              'bg-transparent text-[var(--color-text-3)] cursor-pointer',
              'hover:border-[rgba(var(--danger-6),0.28)] hover:bg-[rgba(var(--danger-6),0.08)] hover:text-danger-6',
              'transition-colors',
            ].join(' ')}
            title={t('knowledge.actions.delete', { defaultValue: '删除' })}
          >
            <Delete theme='outline' size={13} strokeWidth={3} />
          </div>
        </div>
      </div>
    </div>
  );
};

export default KnowledgeCard;
