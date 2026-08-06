/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '@arco-design/web-react';
import { Check, Close, Edit, Magic, Puzzle } from '@icon-park/react';
import { NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import SkillButton from './SkillButton';
import { SkillMissingBadge, SkillSourceBadge, SkillStatusBadge } from './SkillBadges';
import type { SkillEntry } from './unify';

interface SkillListRowProps {
  entry: SkillEntry;
  selected: boolean;
  /** A grant patch is in flight for this row. */
  busy: boolean;
  /** No grant can be changed right now (another patch in flight / no profile). */
  grantDisabled: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onDecide: (accept: boolean) => void;
  onRevoke: () => void;
}

/**
 * One skill, one row. The row is the click target for the detail pane; the
 * controls on the right are the only things that act without opening it —
 * a draft decision (always visible, because a draft is waiting on the user)
 * or a granted capability's on/off switch.
 */
const SkillListRow: React.FC<SkillListRowProps> = ({
  entry,
  selected,
  busy,
  grantDisabled,
  onSelect,
  onEdit,
  onDecide,
  onRevoke,
}) => {
  const { t } = useTranslation();
  const isDraft = entry.kind === 'generated' && entry.status === 'draft';

  const controls = (
    <>
      {isDraft && (
        <>
          <SkillButton
            tone='primary'
            icon={<Check theme='outline' size='12' fill='currentColor' strokeWidth={4} />}
            onClick={() => onDecide(true)}
          >
            {t('nomi.skills.accept', { defaultValue: '采纳' })}
          </SkillButton>
          <SkillButton
            tone='danger'
            icon={<Close theme='outline' size='12' fill='currentColor' strokeWidth={4} />}
            onClick={() => onDecide(false)}
          >
            {t('nomi.skills.reject', { defaultValue: '拒绝' })}
          </SkillButton>
        </>
      )}
      {entry.kind === 'generated' && !isDraft && (
        <SkillButton
          className='opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100'
          icon={<Edit theme='outline' size='12' fill='currentColor' strokeWidth={3} />}
          onClick={onEdit}
        >
          {t('nomi.skills.edit', { defaultValue: '编辑' })}
        </SkillButton>
      )}
      {entry.kind === 'catalog' && (
        // The switch is always ON because the row only exists while the grant
        // does — flipping it off revokes and the row leaves the list. Keyboard
        // events must be swallowed too, or Space would toggle the switch AND
        // open the detail pane through the row's own handler.
        <div
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => event.stopPropagation()}
        >
          <Switch
            size='small'
            className='compact-dark-switch'
            aria-label={t('nomi.skills.revoke', { defaultValue: '取消授予' })}
            checked
            loading={busy}
            disabled={grantDisabled && !busy}
            onChange={() => onRevoke()}
          />
        </div>
      )}
    </>
  );

  return (
    <div
      role='button'
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
      className='group cursor-pointer outline-none'
    >
      <NomiSettingRow
        className={classNames(
          'py-9px transition-colors',
          // fill-1, not fill-2: every fill token is translucent, and the row
          // carries fill-2 badges — hovering to fill-2 would stack the same
          // wash twice. Matches the memory list's row hover.
          selected ? '!bg-primary-1' : 'hover:bg-fill-1'
        )}
        leading={
          entry.kind === 'generated' ? (
            <span className='flex shrink-0 text-primary-6'>
              <Magic theme='outline' size='16' fill='currentColor' strokeWidth={3} />
            </span>
          ) : (
            <span className='flex shrink-0 text-t-tertiary'>
              <Puzzle theme='outline' size='16' fill='currentColor' strokeWidth={3} />
            </span>
          )
        }
        titleClassName={selected ? '!text-primary-6' : undefined}
        title={
          <div className='flex min-w-0 items-center gap-6px'>
            <span className='min-w-0 truncate'>{entry.name}</span>
            <SkillSourceBadge entry={entry} />
            {entry.kind === 'generated' && <SkillStatusBadge status={entry.status} />}
            {entry.kind === 'catalog' && !entry.installed && <SkillMissingBadge />}
          </div>
        }
        description={
          entry.description?.trim() || t('nomi.skills.noDescription', { defaultValue: '这个技能还没有描述' })
        }
        descriptionClassName='line-clamp-2'
        controls={controls}
      />
    </div>
  );
};

export default SkillListRow;
