/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Checkbox } from '@arco-design/web-react';
import { Delete, InboxIn, InboxOut, Pin } from '@icon-park/react';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import type { CompanionMemoryId } from '@/common/types/ids';
import { parseSnippetSegments } from './memorySnippet';
import { MEMORY_KIND_DOT, formatMemoryTime } from './constants';

/**
 * The list is deliberately NOT a stack of cards. Spell out every border side so
 * no inherited shorthand can turn the rows into a boxed grid: one hairline
 * under each row, nothing else.
 */
const ROW_DIVIDER_STYLE: React.CSSProperties = {
  borderTopWidth: 0,
  borderRightWidth: 0,
  borderBottomWidth: 1,
  borderLeftWidth: 0,
  borderBottomStyle: 'solid',
  borderBottomColor: 'var(--color-border-2)',
};

const ROW_LAYOUT_STYLE: React.CSSProperties = {
  gridTemplateColumns: '22px 92px minmax(0, 1fr) 76px 116px 72px',
};

const stop = (event: React.SyntheticEvent) => event.stopPropagation();

interface MemoryListRowProps {
  memory: ICompanionMemory;
  checked: boolean;
  /** The row whose detail pane is open. */
  active: boolean;
  onToggleSelect: (id: CompanionMemoryId) => void;
  onOpen: (memory: ICompanionMemory) => void;
  onTogglePin: (memory: ICompanionMemory) => void;
  onToggleArchive: (memory: ICompanionMemory) => void;
  onDelete: (memory: ICompanionMemory) => void;
}

const MemoryListRow: React.FC<MemoryListRowProps> = ({
  memory,
  checked,
  active,
  onToggleSelect,
  onOpen,
  onTogglePin,
  onToggleArchive,
  onDelete,
}) => {
  const { t } = useTranslation();
  const archived = memory.status === 'archived';

  /** FTS snippet with `<b>` hits, parsed through the whitelist — never innerHTML. */
  const content = memory.snippet ? (
    <>
      {parseSnippetSegments(memory.snippet).map((segment, index) =>
        segment.hit ? (
          <b key={index} className='font-600 text-primary-6'>
            {segment.text}
          </b>
        ) : (
          <React.Fragment key={index}>{segment.text}</React.Fragment>
        )
      )}
    </>
  ) : (
    memory.content
  );

  const iconAction = (
    label: string,
    icon: React.ReactNode,
    onClick: () => void,
    danger?: boolean
  ) => (
    <span
      role='button'
      tabIndex={0}
      aria-label={label}
      title={label}
      onClick={onClick}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onClick();
        }
      }}
      className={[
        'inline-flex cursor-pointer items-center text-t-tertiary transition-colors',
        danger ? 'hover:text-danger-6' : 'hover:text-primary-6',
      ].join(' ')}
    >
      {icon}
    </span>
  );

  return (
    <div
      role='button'
      tabIndex={0}
      onClick={() => onOpen(memory)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          onOpen(memory);
        }
      }}
      className={[
        'group grid min-h-52px min-w-0 cursor-pointer items-center gap-x-8px px-4px py-8px transition-colors duration-150',
        active ? '!bg-primary-1' : 'hover:bg-fill-1',
      ].join(' ')}
      style={{ ...ROW_DIVIDER_STYLE, ...ROW_LAYOUT_STYLE }}
    >
      <div className='shrink-0' onClick={stop}>
        <Checkbox checked={checked} onChange={() => onToggleSelect(memory.memory_id)} />
      </div>

      {/* Kind — a dot plus a quiet label, not a coloured tag. */}
      <span className='flex min-w-0 items-center gap-6px text-12px leading-18px text-t-secondary'>
        <span
          aria-hidden
          className='h-6px w-6px shrink-0 rd-full'
          style={{ background: MEMORY_KIND_DOT[memory.kind] }}
        />
        <span className='truncate'>{t(`nomi.kinds.${memory.kind}`)}</span>
      </span>

      <span
        className={[
          'min-w-0 text-13px leading-20px break-words',
          archived ? 'text-t-tertiary' : active ? '!text-primary-6' : 'text-t-primary',
        ].join(' ')}
        style={{ display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}
      >
        {memory.pinned && (
          <Pin
            theme='filled'
            size='12'
            fill='currentColor'
            className='mr-4px inline-flex text-primary-6'
            style={{ verticalAlign: -1 }}
          />
        )}
        {content}
      </span>

      {/* Strength doubles as the importance read-out: a pinned memory never decays. */}
      <span className='hidden text-12px leading-18px text-t-tertiary tabular-nums lg:block'>
        {`${Math.round(memory.strength * 100)}%`}
      </span>

      <span className='hidden text-12px leading-18px text-t-tertiary tabular-nums xl:block'>
        {formatMemoryTime(memory.updated_at)}
      </span>

      <div
        className='flex shrink-0 items-center justify-end gap-10px opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100'
        onClick={stop}
      >
        {iconAction(
          memory.pinned
            ? t('nomi.memories.unpin', { defaultValue: '取消置顶' })
            : t('nomi.memories.pin', { defaultValue: '置顶（不衰减）' }),
          <Pin theme='outline' size='15' fill='currentColor' strokeWidth={3} />,
          () => onTogglePin(memory)
        )}
        {iconAction(
          archived ? t('nomi.memories.restore', { defaultValue: '恢复' }) : t('nomi.memories.archive', { defaultValue: '归档' }),
          archived ? (
            <InboxOut theme='outline' size='15' fill='currentColor' strokeWidth={3} />
          ) : (
            <InboxIn theme='outline' size='15' fill='currentColor' strokeWidth={3} />
          ),
          () => onToggleArchive(memory)
        )}
        {iconAction(
          t('nomi.memories.delete', { defaultValue: '删除' }),
          <Delete theme='outline' size='15' fill='currentColor' strokeWidth={3} />,
          () => onDelete(memory),
          true
        )}
      </div>
    </div>
  );
};

export default MemoryListRow;
