/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { DndContext, PointerSensor, closestCenter, useSensor, useSensors } from '@dnd-kit/core';
import { SortableContext, useSortable, verticalListSortingStrategy } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import { CSS } from '@dnd-kit/utilities';
import classNames from 'classnames';
import { Delete, Drag, Pic, Plus } from '@icon-park/react';
import { ipcBridge } from '@/common';
import ContentSider from '@/renderer/components/layout/ContentSider';
import InstantHoverTooltip from '@/renderer/components/base/InstantHoverTooltip';
import CompanionAvatar from '@renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import type { CompanionMood } from '@renderer/pages/companion/characters';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';

interface CompanionRowProps {
  companion: ICompanionWithStatus;
  active: boolean;
  onSelect: (id: CompanionId) => void;
  onRequestDelete: (companion: ICompanionWithStatus) => void;
  /** Roving tabindex: only the active row is focusable. */
  tabIndex: number;
  registerRef: (id: CompanionId, el: HTMLDivElement | null) => void;
  onKeyNav: (event: React.KeyboardEvent, id: CompanionId) => void;
}

/**
 * One roster row. 44px tall (taller than the app's 34px nav row because it
 * carries an avatar), otherwise the canonical rail grammar: rd-8px, gap-8px,
 * `!bg-primary-1 !text-primary-6` when selected.
 */
const CompanionRow: React.FC<CompanionRowProps> = ({
  companion,
  active,
  onSelect,
  onRequestDelete,
  tabIndex,
  registerRef,
  onKeyNav,
}) => {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: companion.companion_id,
  });
  const modelReady = companion.model !== null;

  return (
    <div
      ref={(el) => {
        setNodeRef(el);
        registerRef(companion.companion_id, el);
      }}
      role='tab'
      aria-selected={active}
      tabIndex={tabIndex}
      onKeyDown={(event) => onKeyNav(event, companion.companion_id)}
      onClick={() => onSelect(companion.companion_id)}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={classNames(
        'group relative flex items-center gap-8px shrink-0 h-44px rd-8px pl-8px pr-6px cursor-pointer transition-colors box-border outline-none',
        active ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3',
        isDragging && 'opacity-60'
      )}
    >
      <div className='relative shrink-0'>
        <CompanionAvatar
          character={companion.character}
          companionId={companion.companion_id}
          customFigure={customFigureMetaOf(companion)}
          mood={(companion.status.mood as CompanionMood) || 'content'}
          activity='idle'
          size={30}
        />
        {/* Model readiness is the one thing that decides whether this companion can
            talk at all, so it rides the avatar rather than hiding in a tab. */}
        <span
          className='absolute -right-1px -bottom-1px w-8px h-8px rd-full border-2 border-[var(--color-bg-2)]'
          style={{ background: modelReady ? 'rgb(var(--success-6))' : 'rgb(var(--warning-6))' }}
          title={modelReady ? undefined : t('nomi.chat.modelUnset')}
        />
      </div>
      <div className='flex flex-col gap-1px min-w-0 flex-1'>
        <span
          className={classNames('text-13px leading-16px font-600 truncate', active ? '!text-primary-6' : 'text-t-primary')}
        >
          {companion.name}
        </span>
        <span className={classNames('text-11px leading-13px', active ? 'text-primary-6 opacity-70' : 'text-t-tertiary')}>
          Lv{companion.status.level}
        </span>
      </div>
      {/* Row actions live on an opaque tier: they float over text, and the app's
          fill tokens are translucent (see styles/layout.css:12-23). */}
      <div className='shrink-0 flex items-center gap-2px opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity'>
        <InstantHoverTooltip content={t('nomi.companions.reorder', { defaultValue: '拖动调整位置' })}>
          {/* useSortable's `attributes` already supplies role/aria for the drag
              affordance — spread it last so it wins over any local guess. */}
          <div
            aria-label={t('nomi.companions.reorder', { defaultValue: '拖动调整位置' })}
            onClick={(event) => event.stopPropagation()}
            className='flex items-center justify-center w-20px h-20px rd-6px text-t-tertiary hover:text-t-secondary cursor-grab active:cursor-grabbing'
            {...attributes}
            {...listeners}
          >
            <Drag theme='outline' size='13' fill='currentColor' />
          </div>
        </InstantHoverTooltip>
        <InstantHoverTooltip content={t('nomi.settings.deleteCompanion')}>
          <div
            role='button'
            aria-label={t('nomi.settings.deleteCompanion')}
            onClick={(event) => {
              event.stopPropagation();
              onRequestDelete(companion);
            }}
            className='flex items-center justify-center w-20px h-20px rd-6px text-t-tertiary hover:!text-danger-6 hover:bg-[var(--color-bg-1)] transition-colors cursor-pointer'
          >
            <Delete theme='outline' size='13' fill='currentColor' />
          </div>
        </InstantHoverTooltip>
      </div>
    </div>
  );
};

export interface CompanionSidebarProps {
  companions: ICompanionWithStatus[];
  selectedId: CompanionId | null;
  /** True while the 形象库 view owns the workspace. */
  figuresActive: boolean;
  width: number;
  onSelect: (id: CompanionId) => void;
  onOpenFigures: () => void;
  onCreate: () => void;
  onRequestDelete: (companion: ICompanionWithStatus) => void;
  /** New full order, first to last. */
  onReorder: (orderedIds: CompanionId[]) => void;
  resizeHandle?: React.ReactNode;
}

/**
 * The companion roster sidebar — create at the top, the reorderable roster in
 * the middle, the figure library pinned at the bottom.
 *
 * Replaces the former `CompanionSessionRail`, whose name and framing ("会话切换栏")
 * predate chat moving out of this page: it is a roster, not a session switcher.
 */
const CompanionSidebar: React.FC<CompanionSidebarProps> = ({
  companions,
  selectedId,
  figuresActive,
  width,
  onSelect,
  onOpenFigures,
  onCreate,
  onRequestDelete,
  onReorder,
  resizeHandle,
}) => {
  const { t } = useTranslation();
  const rowRefs = useRef(new Map<CompanionId, HTMLDivElement>());
  const registerRef = useCallback((id: CompanionId, el: HTMLDivElement | null) => {
    if (el) rowRefs.current.set(id, el);
    else rowRefs.current.delete(id);
  }, []);

  // A drag must travel a few px before it starts, or every row click would be
  // interpreted as a zero-distance drag and selection would stop working.
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));
  const ids = useMemo(() => companions.map((c) => c.companion_id), [companions]);

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const from = ids.indexOf(active.id as CompanionId);
      const to = ids.indexOf(over.id as CompanionId);
      if (from === -1 || to === -1) return;
      const next = ids.slice();
      next.splice(to, 0, next.splice(from, 1)[0]);
      onReorder(next);
    },
    [ids, onReorder]
  );

  const onKeyNav = useCallback(
    (event: React.KeyboardEvent, id: CompanionId) => {
      const index = ids.indexOf(id);
      let nextIndex: number | null = null;
      if (event.key === 'ArrowDown') nextIndex = Math.min(ids.length - 1, index + 1);
      else if (event.key === 'ArrowUp') nextIndex = Math.max(0, index - 1);
      else if (event.key === 'Home') nextIndex = 0;
      else if (event.key === 'End') nextIndex = ids.length - 1;
      else if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onSelect(id);
        return;
      }
      if (nextIndex === null) return;
      event.preventDefault();
      const nextId = ids[nextIndex];
      onSelect(nextId);
      rowRefs.current.get(nextId)?.focus();
    },
    [ids, onSelect]
  );

  return (
    <ContentSider
      width={width}
      ariaLabel={t('nomi.title')}
      resizeHandle={resizeHandle}
      header={
        <div className='px-8px pt-12px pb-8px'>
          {/* The soft primary CTA (12% tint, not a saturated fill) is the app's
              most elegant call to action — see KnowledgeListPage. */}
          <div
            role='button'
            tabIndex={0}
            onClick={onCreate}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                onCreate();
              }
            }}
            className='flex items-center justify-center gap-6px h-36px rd-full px-14px cursor-pointer font-700 text-13px text-[var(--color-text-1)] bg-[rgba(var(--primary-6),0.12)] hover:bg-[rgba(var(--primary-6),0.18)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] transition-colors box-border outline-none'
          >
            <Plus theme='outline' size='15' fill='currentColor' />
            <span className='truncate'>{t('nomi.companions.create')}</span>
          </div>
        </div>
      }
      footer={
        <div className='px-8px pb-8px pt-4px'>
          <div
            role='button'
            tabIndex={0}
            aria-selected={figuresActive}
            onClick={onOpenFigures}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                onOpenFigures();
              }
            }}
            className={classNames(
              'flex items-center gap-8px h-34px rd-8px px-10px cursor-pointer transition-colors box-border outline-none',
              figuresActive ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
            )}
          >
            <span
              className={classNames(
                'shrink-0 flex items-center justify-center size-22px',
                figuresActive ? 'text-primary-6' : 'text-t-secondary'
              )}
            >
              <Pic theme='outline' size='16' fill='currentColor' strokeWidth={3} />
            </span>
            <span
              className={classNames(
                'text-14px font-500 truncate',
                figuresActive ? '!text-primary-6' : 'text-t-primary'
              )}
            >
              {t('nomi.customFigure.libraryTitle')}
            </span>
          </div>
        </div>
      }
    >
      <div className='flex flex-col gap-2px px-8px pb-8px' role='tablist' aria-orientation='vertical'>
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext items={ids} strategy={verticalListSortingStrategy}>
            {companions.map((companion) => (
              <CompanionRow
                key={companion.companion_id}
                companion={companion}
                active={!figuresActive && companion.companion_id === selectedId}
                onSelect={onSelect}
                onRequestDelete={onRequestDelete}
                tabIndex={companion.companion_id === selectedId ? 0 : -1}
                registerRef={registerRef}
                onKeyNav={onKeyNav}
              />
            ))}
          </SortableContext>
        </DndContext>
        {companions.length === 0 && (
          <div className='px-6px py-24px text-center text-12px leading-18px text-t-tertiary'>
            {t('nomi.companions.empty')}
          </div>
        )}
      </div>
    </ContentSider>
  );
};

export default CompanionSidebar;

/** Delete one companion after a danger confirm. Shared by the sidebar and 其他. */
export const confirmDeleteCompanion = async (
  companion: { companion_id: CompanionId; name: string },
  t: (key: string, options?: Record<string, unknown>) => string,
  onDeleted: (companionId: CompanionId) => void
): Promise<void> => {
  try {
    await ipcBridge.companion.deleteCompanion.invoke({ companion_id: companion.companion_id });
    Message.success(t('nomi.settings.deleted', { companionName: companion.name }));
    onDeleted(companion.companion_id);
  } catch (error) {
    Message.error(String(error));
  }
};
