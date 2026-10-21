/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Button, Dropdown, Menu, Spin, Tooltip } from '@arco-design/web-react';
import { Delete, Earth, EditTwo, FolderOpen, MoreOne } from '@icon-park/react';
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { formatSize } from './useKnowledge';
import { getKindConfig } from './knowledgeKind';
import styles from './KnowledgeCard.module.css';

export interface KnowledgeCardProps {
  base: IKnowledgeBase;
  tagMap?: Record<string, IKnowledgeTag>;
  onOpen?: (base: IKnowledgeBase) => void;
  onPrefetch?: (base: IKnowledgeBase) => void;
  opening?: boolean;
  onEdit?: (base: IKnowledgeBase) => void;
  onDelete?: (base: IKnowledgeBase, e: React.MouseEvent) => void;
}

const MAX_VISIBLE_TAGS = 2;

function TagChips({ tags }: { tags: IKnowledgeTag[] }) {
  if (!tags.length) return null;
  const visibleTags = tags.slice(0, MAX_VISIBLE_TAGS);
  const overflowCount = tags.length - visibleTags.length;

  return (
    <span className={`knowledge-card-tags ${styles.tags}`}>
      {visibleTags.map((tag) => (
        <span key={tag.key} className={styles.tag}>
          {tag.color && <i className={styles.tagDot} style={{ background: tag.color }} aria-hidden='true' />}
          <span className={styles.tagLabel}>{tag.label}</span>
        </span>
      ))}
      {overflowCount > 0 && <span className={styles.tagOverflow}>+{overflowCount}</span>}
    </span>
  );
}

function formatRelativeTime(epochMs: number, t: TFunction): string {
  const seconds = Math.floor((Date.now() - epochMs) / 1000);
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

/** Fixed-height catalog card; opening and menu controls are separate buttons. */
export const KnowledgeCard: React.FC<KnowledgeCardProps> = ({
  base,
  tagMap,
  onOpen,
  onPrefetch,
  opening = false,
  onEdit,
  onDelete,
}) => {
  const { t } = useTranslation();
  const prefetchTimerRef = React.useRef<number | null>(null);
  const [menuVisible, setMenuVisible] = React.useState(false);
  const cancelPrefetch = () => {
    if (prefetchTimerRef.current === null) return;
    window.clearTimeout(prefetchTimerRef.current);
    prefetchTimerRef.current = null;
  };
  const prefetchNow = () => {
    cancelPrefetch();
    if (!opening) onPrefetch?.(base);
  };
  const schedulePrefetch = () => {
    if (opening || !onPrefetch || prefetchTimerRef.current !== null) return;
    prefetchTimerRef.current = window.setTimeout(() => {
      prefetchTimerRef.current = null;
      onPrefetch(base);
    }, 120);
  };
  React.useEffect(() => cancelPrefetch, []);

  const kindConfig = getKindConfig(base.kind, t, 'neutral');
  const sourceLabel = [
    kindConfig.label,
    base.source?.mode === 'live'
      ? t('knowledge.card.modeLive', { defaultValue: '实时' })
      : base.source?.mode === 'snapshot'
        ? t('knowledge.card.modeSnapshot', { defaultValue: '快照' })
        : null,
  ].filter(Boolean).join(' · ');
  const resolvedTags = base.tags
    .map((key) => tagMap?.[key])
    .filter((tag): tag is IKnowledgeTag => Boolean(tag));
  const metaItems = [
    t('knowledge.card.fileCount', { count: base.file_count, defaultValue: '{{count}} 篇' }),
    formatSize(base.total_size),
    formatRelativeTime(base.updated_at, t),
  ];
  const rootMissingLabel = t('knowledge.card.rootMissing', { defaultValue: '目录不可用' });
  const tooltipContent = (
    <div className={styles.tooltip}>
      <strong>{base.name}</strong>
      <div>{sourceLabel}</div>
      {!base.root_exists && <div>{rootMissingLabel}</div>}
      {base.description && <div>{base.description}</div>}
      {resolvedTags.length > 0 && <div>{resolvedTags.map((tag) => tag.label).join(' · ')}</div>}
    </div>
  );
  const iconProps = { theme: 'outline' as const, size: 15, strokeWidth: 3, fill: 'currentColor' };

  return (
    <article
      className={styles.card}
      aria-busy={opening}
      data-knowledge-base-id={base.knowledge_base_id}
      data-knowledge-opening={opening ? 'true' : 'false'}
    >
      <Tooltip
        content={tooltipContent}
        position='top'
        trigger={['hover', 'focus']}
        disabled={opening || menuVisible}
        triggerProps={{ mouseEnterDelay: 400 }}
      >
        <button
          type='button'
          className={styles.openButton}
          disabled={opening}
          aria-label={`${t('knowledge.card.actionOpen', { defaultValue: '打开' })}: ${base.name}`}
          onMouseEnter={schedulePrefetch}
          onMouseLeave={cancelPrefetch}
          onFocus={prefetchNow}
          onPointerDown={prefetchNow}
          onClick={() => {
            if (!opening) onOpen?.(base);
          }}
          onKeyDown={(event) => {
            if (!opening && (event.key === 'Enter' || event.key === ' ')) {
              event.preventDefault();
              onOpen?.(base);
            }
          }}
        >
          <span className={styles.heading}>
            <span className={styles.kindIcon} aria-hidden='true'>
              {base.kind === 'local' ? <FolderOpen {...iconProps} /> : base.kind === 'web' ? <Earth {...iconProps} /> : <EditTwo {...iconProps} />}
            </span>
            <span className={styles.name}>{base.name}</span>
            <span className={styles.source}>{sourceLabel}</span>
          </span>
          <span className={styles.body}>
            <span className={`${styles.description} ${resolvedTags.length > 0 ? styles.singleLine : ''}`}>
              {!base.root_exists && <span className={`knowledge-card-root-missing ${styles.rootMissing}`}>{rootMissingLabel}</span>}
              {base.description}
            </span>
            <TagChips tags={resolvedTags} />
          </span>
          <span className={`knowledge-card-footer knowledge-card-meta ${styles.meta}`}>
            {metaItems.map((item, index) => <span key={index}>{item}</span>)}
          </span>
        </button>
      </Tooltip>
      {(onEdit || onDelete) && (
        <Dropdown
          trigger='click'
          position='br'
          disabled={opening}
          popupVisible={!opening && menuVisible}
          onVisibleChange={setMenuVisible}
          droplist={
            <Menu selectable={false}>
              {onEdit && (
                <Menu.Item key='edit' disabled={opening} onClick={() => {
                  if (opening) return;
                  setMenuVisible(false);
                  onEdit(base);
                }}>
                  <span className={styles.menuItem}><EditTwo {...iconProps} />{t('knowledge.card.actionEdit', { defaultValue: '编辑' })}</span>
                </Menu.Item>
              )}
              {onDelete && (
                <Menu.Item key='delete' disabled={opening} onClick={(event) => {
                  event.stopPropagation();
                  if (opening) return;
                  setMenuVisible(false);
                  onDelete(base, event);
                }}>
                  <span className={`${styles.menuItem} ${styles.deleteItem}`}><Delete {...iconProps} />{t('knowledge.actions.delete', { defaultValue: '删除' })}</span>
                </Menu.Item>
              )}
            </Menu>
          }
        >
          <Button
            type='text'
            size='mini'
            className={styles.moreButton}
            disabled={opening}
            aria-label={`${base.name}: ${t('knowledge.card.actionMore', { defaultValue: '更多' })}`}
            aria-haspopup='menu'
            aria-expanded={!opening && menuVisible}
            icon={<MoreOne {...iconProps} />}
          />
        </Dropdown>
      )}
      {opening && (
        <div className={styles.opening} role='status' aria-live='polite'>
          <Spin size={16} />
          <span>{t('knowledge.openingNamed', { name: base.name, defaultValue: '正在打开“{{name}}”…' })}</span>
        </div>
      )}
    </article>
  );
};

export default KnowledgeCard;
