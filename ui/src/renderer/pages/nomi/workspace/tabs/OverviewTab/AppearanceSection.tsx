/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Progress, Switch } from '@arco-design/web-react';
import { Platte } from '@icon-park/react';
import type { ICompanionProfile, ICompanionStatus } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import NomiInput from '@/renderer/components/base/NomiInput';
import CompanionAvatar from '@renderer/pages/companion/CompanionAvatar';
import { CUSTOM_CHARACTER_ID, getCharacter } from '@renderer/pages/companion/characters';
import type { CompanionMood, CustomFigureMeta } from '@renderer/pages/companion/characters';
import { useFigures } from '@renderer/pages/nomi/useFigures';
import type { CompanionHandle } from '../../types';
import RowAction from './RowAction';
import { useDebouncedText } from './useDebouncedText';

interface AppearanceSectionProps {
  profile: ICompanionProfile;
  status: ICompanionStatus;
  patchCompanion: CompanionHandle['patchCompanion'];
  /** Parsed `appearance.custom_figure`, or null for a built-in character. */
  figure: CustomFigureMeta | null;
  /** Opens the figure detail pane (picker + size). */
  onEditFigure: () => void;
  /** The figure pane is currently open — the action reads as pressed. */
  figurePaneOpen: boolean;
}

/**
 * 伙伴形象 — who this companion is on the desktop: its name, the figure it wears,
 * whether it is shown at all, and how far it has grown.
 *
 * The growth row deliberately shows level / XP / mood ONLY. The old overview also
 * printed 记忆 / 新建议 / 专精技能 counts here, but those come from the shared
 * (cross-companion) store and read as if they belonged to this companion — one of
 * the bugs this redesign removes.
 */
const AppearanceSection: React.FC<AppearanceSectionProps> = ({
  profile,
  status,
  patchCompanion,
  figure,
  onEditFigure,
  figurePaneOpen,
}) => {
  const { t } = useTranslation();
  const { figures } = useFigures();

  const [nameDraft, onNameChange] = useDebouncedText(profile.name, (value) => {
    const name = value.trim();
    if (!name || name === profile.name) return;
    void patchCompanion({ name }).catch((e) => Message.error(String(e)));
  });

  // What the user currently wears: a library figure's own name when custom,
  // otherwise the built-in character's display name.
  const lookName =
    profile.character === CUSTOM_CHARACTER_ID
      ? (figures.find((f) => f.figure_id === figure?.figureId)?.name ??
        t('nomi.customFigure.cardLabel', { defaultValue: '自定义形象' }))
      : t(`nomi.characters.${getCharacter(profile.character).nameKey}.name`);

  // Lv = floor(sqrt(xp / 100)) + 1 ⇒ level L spans [(L-1)²·100, L²·100).
  const level = Math.max(1, status.level);
  const levelBase = (level - 1) ** 2 * 100;
  const levelNext = level ** 2 * 100;
  const levelPct = Math.min(100, Math.max(0, Math.round(((status.xp - levelBase) / Math.max(1, levelNext - levelBase)) * 100)));

  return (
    <NomiSettingSection
      title={t('nomi.overview.appearanceSection', { defaultValue: '伙伴形象' })}
      description={t('nomi.overview.appearanceSectionHint', { defaultValue: '名字、桌面上的样子，以及它陪你走到了哪一步' })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.settings.name', { defaultValue: '名字' })}
          description={t('nomi.settings.nameHint', { defaultValue: '伙伴的称呼，会出现在聊天与桌面伙伴里' })}
          controls={
            <NomiInput
              contentFit
              value={nameDraft}
              onChange={onNameChange}
              // A companion must have a name, so an emptied field commits nothing.
              // Without this the box would sit blank forever (the debounce source
              // never changes, so the draft never re-syncs) and look like a saved
              // nameless companion. Snap back to the real name on blur instead.
              onBlur={() => {
                if (!nameDraft.trim()) onNameChange(profile.name);
              }}
              maxLength={30}
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.settings.character', { defaultValue: '桌面形象' })}
          description={t('nomi.overview.figureCurrent', { defaultValue: '当前：{{look}}', look: lookName })}
          controls={
            <>
              <span className='shrink-0 flex items-center justify-center w-44px h-44px rd-8px bg-fill-2 overflow-hidden'>
                <CompanionAvatar
                  character={profile.character}
                  companionId={profile.companion_id}
                  customFigure={figure}
                  mood={(status.mood as CompanionMood) || 'content'}
                  activity='idle'
                  size={40}
                />
              </span>
              <RowAction active={figurePaneOpen} onClick={onEditFigure}>
                <Platte theme='outline' size='14' fill='currentColor' strokeWidth={3} />
                {t('nomi.overview.changeFigure', { defaultValue: '更换形象' })}
              </RowAction>
            </>
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.deskVisible', { defaultValue: '桌面显示' })}
          description={t('nomi.settings.companionEnabledHint', {
            defaultValue: '在桌面显示你的桌面伙伴（透明置顶小窗，仅桌面端）',
          })}
          controls={
            <>
              <span className='shrink-0 text-13px text-t-secondary'>
                {profile.appearance.companion_enabled
                  ? t('nomi.overview.companionOn', { defaultValue: '已显示' })
                  : t('nomi.overview.companionOff', { defaultValue: '已隐藏' })}
              </span>
              <Switch
                size='small'
                className='compact-dark-switch shrink-0'
                checked={profile.appearance.companion_enabled}
                onChange={(companion_enabled) => void patchCompanion({ appearance: { companion_enabled } })}
              />
            </>
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.growthLevel', { defaultValue: '成长等级' })}
          description={t('nomi.overview.growthLevelHint', { defaultValue: '相处越久等级越高，它会越跟得上你的节奏' })}
          controls={
            <>
              <span className='shrink-0 text-13px font-500 text-t-primary'>
                {`Lv${level} · ${t(`nomi.levels.l${Math.min(level, 5)}`)}`}
              </span>
              <span className='shrink-0 rd-full bg-fill-2 px-8px py-2px text-11px text-t-secondary'>
                {t(`nomi.moods.${status.mood}`, { defaultValue: status.mood })}
              </span>
            </>
          }
          footer={
            <div className='flex items-center gap-10px'>
              <Progress className='min-w-0 flex-1' percent={levelPct} showText={false} color='var(--color-primary)' />
              <span className='shrink-0 text-11px text-t-tertiary'>
                {t('nomi.overview.xpProgress', { defaultValue: '{{xp}} / {{next}} XP', xp: status.xp, next: levelNext })}
              </span>
            </div>
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default AppearanceSection;
