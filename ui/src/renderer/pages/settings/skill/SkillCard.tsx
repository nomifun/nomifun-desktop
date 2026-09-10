/**
 * SkillCard — A grid item for the Skills Hub with a rounded bordered surface,
 * soft hover lift, fixed two-line description, and hover actions.
 *   - a deterministic letter avatar (shared getAvatarColorClass), or a Lightning
 *     glyph for auto-injected skills
 *   - a source badge: Built-in / Custom / Auto-injected
 *   - NO enable switch (skills aren't toggled here)
 *   - hover footer: Delete (custom only)
 * The whole card is clickable → onOpenDetails.
 *
 * Theme variables only (the avatar hex palette is the documented exception);
 * `<div onClick>` for clickables (no <button>, to dodge the WebView2 black box).
 */
import type { SkillInfo } from '@/common/types/skill';
import { resolveSkillDisplay } from './skillDisplay';
import { getAvatarColorClass, normalizeTestId } from './skillPresentation';
import { Tag } from '@arco-design/web-react';
import { Delete, Lightning } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

type SkillCardProps = {
  skill: SkillInfo;
  localeKey: string;
  /** True when the skill name is in the built-in auto-inject set (parent-supplied). */
  isAutoInjected: boolean;
  onOpenDetails: (skill: SkillInfo) => void;
  onDelete: (skill: SkillInfo) => void;
  highlighted?: boolean;
  cardRef?: (el: HTMLDivElement | null) => void;
};

/** Source badge — one quiet pill per source, color-coded by semantic. */
const SourceBadge: React.FC<{ skill: SkillInfo; isAutoInjected: boolean }> = ({ skill, isAutoInjected }) => {
  const { t } = useTranslation();

  if (isAutoInjected) {
    return (
      <Tag
        size='small'
        bordered={false}
        className='!flex-shrink-0 !h-14px !text-9px !leading-12px !px-5px !py-0 !rounded-5px !bg-[rgba(var(--success-6),0.1)] !text-success-6'
      >
        {t('settings.skillsHub.sourceAuto', { defaultValue: 'Auto' })}
      </Tag>
    );
  }
  if (skill.source === 'custom') {
    return (
      <Tag
        size='small'
        bordered={false}
        className='!flex-shrink-0 !h-14px !text-9px !leading-12px !px-5px !py-0 !rounded-5px !bg-[rgba(var(--orange-6),0.1)] !text-[rgba(var(--orange-6),1)]'
      >
        {t('settings.skillsHub.custom', { defaultValue: 'Custom' })}
      </Tag>
    );
  }
  return (
    <Tag
      size='small'
      bordered={false}
      className='!flex-shrink-0 !h-14px !text-9px !leading-12px !px-5px !py-0 !rounded-5px !bg-primary-1 !text-primary-6'
    >
      {t('settings.skillsHub.builtin', { defaultValue: 'Built-in' })}
    </Tag>
  );
};

const SkillCard: React.FC<SkillCardProps> = ({
  skill,
  localeKey,
  isAutoInjected,
  onOpenDetails,
  onDelete,
  highlighted = false,
  cardRef,
}) => {
  const { t } = useTranslation();
  const testId = normalizeTestId(skill.name);
  const display = resolveSkillDisplay(skill, localeKey);

  const canDelete = skill.source === 'custom';

  return (
    <div
      ref={cardRef}
      data-testid={`skill-card-${testId}`}
      onClick={() => onOpenDetails(skill)}
      role='button'
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpenDetails(skill);
        }
      }}
      className={[
        'group relative flex flex-col rounded-16px border border-solid p-12px pb-34px cursor-pointer outline-none',
        'transition-all duration-180',
        highlighted
          ? 'border-primary-5 bg-[var(--color-primary-light-1)] shadow-[0_0_0_3px_rgba(var(--primary-6),0.12)]'
          : 'border-[var(--color-border-2)] bg-[var(--color-bg-2)] hover:border-[var(--color-primary-light-4)] hover:shadow-[0_4px_16px_rgba(0,0,0,0.06)]',
      ].join(' ')}
    >
      {/* Header: avatar + name/badge */}
      <div className='flex items-center gap-10px'>
        {isAutoInjected ? (
          <div className='flex-shrink-0 w-36px h-36px rounded-10px flex items-center justify-center bg-[rgba(var(--success-6),0.1)] shadow-sm'>
            <Lightning theme='filled' size={18} fill='rgb(var(--success-6))' />
          </div>
        ) : (
          <div
            className={`flex-shrink-0 w-36px h-36px rounded-10px flex items-center justify-center font-bold text-15px shadow-sm uppercase ${getAvatarColorClass(skill.name)}`}
          >
            {skill.name.charAt(0).toUpperCase()}
          </div>
        )}
        <div className='flex h-36px min-w-0 flex-1 flex-col justify-between'>
          <div className='flex h-20px min-w-0 items-center'>
            <span
              className='min-w-0 flex-1 truncate text-14px font-medium leading-20px text-[var(--color-text-1)]'
              title={display.name}
            >
              {display.name}
            </span>
          </div>
          <div className='flex h-14px items-center'>
            <SourceBadge skill={skill} isAutoInjected={isAutoInjected} />
          </div>
        </div>
      </div>

      {/* Description — fixed 2-line clamp so cards stay even-height */}
      <div
        className='mt-6px text-12px leading-18px text-[var(--color-text-3)] min-h-[36px]'
        title={display.description || undefined}
        style={{
          display: '-webkit-box',
          WebkitLineClamp: 2,
          WebkitBoxOrient: 'vertical',
          overflow: 'hidden',
        }}
      >
        {display.description || t('settings.skillsHub.noDescription', { defaultValue: 'No description provided.' })}
      </div>

      {/* Hover footer — quiet action links, revealed on card hover */}
      <div
        className='absolute bottom-10px right-12px flex items-center justify-end gap-12px opacity-0 group-hover:opacity-100 transition-opacity duration-180'
        onClick={(e) => e.stopPropagation()}
      >
        {canDelete && (
          <span
            role='button'
            tabIndex={0}
            data-testid={`btn-delete-${testId}`}
            onClick={() => onDelete(skill)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                e.stopPropagation();
                onDelete(skill);
              }
            }}
            className='inline-flex items-center gap-4px text-12px text-[var(--color-text-3)] cursor-pointer hover:text-danger-6 transition-colors'
          >
            <Delete theme='outline' size={13} strokeWidth={3} />
            {t('common.delete', { defaultValue: 'Delete' })}
          </span>
        )}
      </div>
    </div>
  );
};

export default SkillCard;
