/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { Message, Modal } from '@arco-design/web-react';
import { Delete, Plus } from '@icon-park/react';
import { getBaseUrl, isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IFigureMeta } from '@/common/adapter/ipcBridge';
import type { FigureId } from '@/common/types/ids';
import { CHARACTERS, CUSTOM_CHARACTER_ID } from '@renderer/pages/companion/characters';
import type { CompanionMood } from '@renderer/pages/companion/characters';
import { figureImageUrlOf } from '@renderer/pages/companion/characters/customMeta';
import { CHECKER_BG } from './CustomFigureWizard/FrameStep';
import CustomFigureWizard from './CustomFigureWizard';
import { FigureActionButton, FigureActionSurface, FigureActionVeil } from './FigureCardActions';
import { useFigures, useFiguresInUse } from './useFigures';

/**
 * Character + figure picker. Shows the built-in roster, the user's reusable
 * **figure library** (each selectable / deletable), and a "new custom figure"
 * card that opens the DIY wizard immediately (no confirm-first detour). The
 * wizard creates a library figure and auto-selects it on completion.
 */
const CharacterPicker: React.FC<{
  /** Selected character id ('mochi'…'custom'). */
  value: string;
  /** Selected library figure id when `value === 'custom'`. */
  figureId?: FigureId;
  /** Use a denser card grid when embedded directly in the settings page. */
  compact?: boolean;
  /** A built-in roster character was chosen. */
  onSelectCharacter: (id: string) => void;
  /** A library figure was chosen (or just created). */
  onSelectFigure: (figure: IFigureMeta) => void;
}> = ({ value, figureId, compact = false, onSelectCharacter, onSelectFigure }) => {
  const { t } = useTranslation();
  const [hovered, setHovered] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const { figures, remove, add } = useFigures();
  const inUse = useFiguresInUse();
  const base = getBaseUrl();

  const confirmDelete = (fig: IFigureMeta): void => {
    Modal.confirm({
      title: t('nomi.customFigure.libraryTitle'),
      content: t('nomi.customFigure.deleteConfirm'),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        try {
          await remove(fig.figure_id);
        } catch (e) {
          const msg =
            isBackendHttpError(e) && e.code === 'CONFLICT'
              ? t('nomi.customFigure.inUseCannotDelete')
              : `${t('nomi.customFigure.deleteFailed')}: ${String(e)}`;
          Message.error(msg);
        }
      },
    });
  };

  return (
    <>
      <div
        className={classNames(
          'grid',
          compact
            ? 'grid-cols-4 gap-8px max-[1040px]:grid-cols-3 max-[720px]:grid-cols-2'
            : 'grid-cols-3 gap-10px max-[720px]:grid-cols-2'
        )}
      >
        {CHARACTERS.map((c) => {
          const active = c.id === value && value !== CUSTOM_CHARACTER_ID;
          const mood: CompanionMood = hovered === c.id ? 'excited' : 'content';
          return (
            <div
              key={c.id}
              onClick={() => onSelectCharacter(c.id)}
              onMouseEnter={() => setHovered(c.id)}
              onMouseLeave={() => setHovered((h) => (h === c.id ? null : h))}
              className={classNames(
                'flex flex-col items-center cursor-pointer transition-all border-solid',
                compact
                  ? 'gap-4px rd-10px px-8px pt-8px pb-7px border'
                  // 宽度必须写 border-2px：`border-2` 是 --bg-2 颜色，而且生成顺序在
                  // border-[var(--color-primary)] 之后，会把下面那条选中色整条盖掉——
                  // 非紧凑卡片因此既是 3px 的 medium 边框，又永远显示不出选中态。
                  // `border-2` is a colour that outranks the active ring below it.
                  : 'gap-6px rd-12px px-10px pt-12px pb-10px border-2px',
                active ? 'border-[var(--color-primary)] !bg-primary-1 shadow-[0_4px_14px_rgba(var(--primary-rgb),0.25)]' : 'border-transparent bg-fill-2 hover:bg-fill-3'
              )}
            >
              <c.Component mood={mood} activity='idle' size={compact ? 64 : 84} />
              <div className='flex items-center gap-6px'>
                <span className='flex shrink-0 overflow-hidden rd-full w-14px h-14px border border-solid border-[var(--color-border-2)]'>
                  <span className='w-1/2 h-full' style={{ background: c.palette[0] }} />
                  <span className='w-1/2 h-full' style={{ background: c.palette[1] }} />
                </span>
                <span className={classNames('text-13px font-600', active ? 'text-[var(--color-primary)]' : 'text-t-primary')}>
                  {t(`nomi.characters.${c.nameKey}.name`)}
                </span>
              </div>
              <span className='text-11px text-t-tertiary text-center leading-snug'>
                {t(`nomi.characters.${c.nameKey}.style`)}
              </span>
            </div>
          );
        })}

        {/* Library figures — each selectable, delete on hover. */}
        {figures.map((fig) => {
          const active = value === CUSTOM_CHARACTER_ID && figureId === fig.figure_id;
          const used = inUse.has(fig.figure_id);
          return (
            <div
              key={fig.figure_id}
              onClick={() => onSelectFigure(fig)}
              className={classNames(
                'group relative flex flex-col items-center cursor-pointer overflow-hidden transition-all border-solid',
                compact
                  ? 'gap-4px rd-10px px-8px pt-8px pb-7px border'
                  : 'gap-6px rd-12px px-10px pt-12px pb-10px border-2px',
                active ? 'border-[var(--color-primary)] !bg-primary-1 shadow-[0_4px_14px_rgba(var(--primary-rgb),0.25)]' : 'border-transparent bg-fill-2 hover:bg-fill-3'
              )}
            >
              <FigureActionVeil className={compact ? 'h-36px' : 'h-44px'} />
              <FigureActionSurface>
                <FigureActionButton
                  tone='danger'
                  disabled={used}
                  title={used ? t('nomi.customFigure.inUseCannotDelete') : t('nomi.customFigure.delete')}
                  ariaLabel={used ? t('nomi.customFigure.inUseCannotDelete') : t('nomi.customFigure.delete')}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!used) confirmDelete(fig);
                  }}
                >
                  <Delete theme='outline' size='13' fill='currentColor' />
                </FigureActionButton>
              </FigureActionSurface>
              <span
                className={classNames(
                  'flex items-center justify-center w-full rd-8px overflow-hidden',
                  compact ? 'h-64px' : 'h-84px'
                )}
                style={CHECKER_BG}
              >
                <img
                  src={figureImageUrlOf(base, fig.figure_id, fig.created_at)}
                  alt={fig.name}
                  draggable={false}
                  className={classNames('max-w-full object-contain', compact ? 'max-h-64px' : 'max-h-84px')}
                />
              </span>
              <span className={classNames('text-13px font-600 truncate max-w-full', active ? 'text-[var(--color-primary)]' : 'text-t-primary')}>
                {fig.name}
              </span>
              <span className='text-11px text-t-tertiary'>{t('nomi.customFigure.cardLabel')}</span>
            </div>
          );
        })}

        {/* New / import custom figure — opens the wizard immediately. */}
        <div
          onClick={() => setWizardOpen(true)}
          className={classNames(
            'flex flex-col items-center justify-center cursor-pointer transition-all border-dashed border-[var(--color-border-2)] bg-fill-2 hover:bg-fill-3 hover:border-[var(--color-primary)]',
            compact
              ? 'gap-4px rd-10px px-8px pt-8px pb-7px border'
              : 'gap-6px rd-12px px-10px pt-12px pb-10px border-2px'
          )}
        >
          <span
            className={classNames(
              'flex items-center justify-center text-t-tertiary',
              compact ? 'h-64px text-26px' : 'h-84px text-32px'
            )}
          >
            <Plus theme='outline' size='14' fill='currentColor' />
          </span>
          <span className='text-13px font-600 text-t-primary'>{t('nomi.customFigure.createNew')}</span>
          <span className='text-11px text-t-tertiary text-center leading-snug'>{t('nomi.customFigure.cardHint')}</span>
        </div>
      </div>

      <CustomFigureWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onDone={(figure) => {
          setWizardOpen(false);
          add(figure);
          onSelectFigure(figure);
        }}
      />
    </>
  );
};

export default CharacterPicker;
