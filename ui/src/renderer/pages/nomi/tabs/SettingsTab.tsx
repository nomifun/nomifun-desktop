/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message, Modal, Spin, TimePicker } from '@arco-design/web-react';
import { Attention } from '@icon-park/react';
import { ipcBridge } from '@/common';
import NomiInput from '@/renderer/components/base/NomiInput';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { CUSTOM_CHARACTER_ID } from '@renderer/pages/companion/characters';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import CharacterPicker from '../CharacterPicker';
import { figureToCustomPatch } from '../useFigures';
import type { useCompanion } from '../useNomi';
import PresetApplyControl from '@/renderer/components/preset/PresetApplyControl';
import type { CompanionId } from '@/common/types/ids';

interface Props {
  companion: ReturnType<typeof useCompanion>;
  /** Called after this companion was deleted so the page can switch selection. */
  onDeleted: (companionId: CompanionId) => void;
}

/**
 * Debounced text editing over an optimistically-patched source value: local
 * draft follows keystrokes, the commit fires after `delay` ms of quiet.
 */
const useDebouncedText = (source: string, commit: (value: string) => void, delay = 500) => {
  const [draft, setDraft] = useState(source);
  const timerRef = useRef<number | undefined>(undefined);
  const commitRef = useRef(commit);
  commitRef.current = commit;

  useEffect(() => {
    setDraft(source);
  }, [source]);
  useEffect(() => () => window.clearTimeout(timerRef.current), []);

  const onChange = useCallback(
    (value: string) => {
      setDraft(value);
      window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => commitRef.current(value), delay);
    },
    [delay]
  );

  return [draft, onChange] as const;
};

const SettingsTab: React.FC<Props> = ({ companion, onDeleted }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;

  const [nameDraft, onNameChange] = useDebouncedText(profile?.name ?? '', (value) => {
    const name = value.trim();
    if (!name || name === profile?.name) return;
    void patchCompanion({ name }).catch((e) => Message.error(String(e)));
  });

  const [customDraft, onCustomChange] = useDebouncedText(profile?.persona.custom ?? '', (custom) => {
    if (custom === profile?.persona.custom) return;
    void patchCompanion({ persona: { custom } }).catch((e) => Message.error(String(e)));
  });

  const confirmDelete = useCallback(() => {
    if (!profile) return;
    const companionName = profile.name;
    Modal.confirm({
      title: t('nomi.settings.deleteConfirmTitle'),
      content: t('nomi.settings.deleteConfirmBody', { companionName }),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        try {
          await ipcBridge.companion.deleteCompanion.invoke({ companion_id: profile.companion_id });
          Message.success(t('nomi.settings.deleted', { companionName }));
          onDeleted(profile.companion_id);
        } catch (e) {
          Message.error(String(e));
        }
      },
    });
  }, [profile, onDeleted, t]);

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  const companionName = profile.name;

  return (
    <div className='flex flex-col gap-22px py-8px'>
      <NomiSettingSection title={t('nomi.settings.basicSection')}>
        <NomiSettingList>
          <NomiSettingRow
            title={t('nomi.settings.name')}
            description={t('nomi.settings.nameHint')}
            controls={<NomiInput contentFit value={nameDraft} onChange={onNameChange} maxLength={30} />}
          />
          <NomiSettingRow
            title={t('nomi.settings.preset')}
            description={t('nomi.settings.presetHint')}
            controls={
              <PresetApplyControl
                compact
                target='companion'
                appliedPreset={profile.applied_preset}
                onApply={async (presetId, locale) => {
                  await ipcBridge.companion.applyPreset.invoke({
                    companion_id: profile.companion_id,
                    preset_id: presetId,
                    locale,
                  });
                  await companion.refresh();
                }}
              />
            }
          />
        </NomiSettingList>
      </NomiSettingSection>

      <NomiSettingSection
        title={t('nomi.settings.character')}
        description={t('nomi.settings.characterHint')}
      >
        <div className='overflow-hidden rd-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] p-8px'>
          <CharacterPicker
            compact
            value={profile.character || 'mochi'}
            figureId={customFigureMetaOf(profile)?.figureId}
            onSelectCharacter={(character) => void patchCompanion({ character, appearance: { custom_figure: null } })}
            onSelectFigure={(fig) =>
              void patchCompanion({
                character: CUSTOM_CHARACTER_ID,
                appearance: { custom_figure: figureToCustomPatch(fig) },
              })
            }
          />
        </div>
      </NomiSettingSection>

      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.settings.persona')}
          description={t('nomi.settings.personaHint', { companionName })}
          controls={
            <NomiSelect
              contentFit
              contentMaxWidth={260}
              value={profile.persona.preset}
              onChange={(preset: string) => void patchCompanion({ persona: { preset } })}
            >
              <NomiSelect.Option value='lively'>{t('nomi.settings.personaLively')}</NomiSelect.Option>
              <NomiSelect.Option value='calm'>{t('nomi.settings.personaCalm')}</NomiSelect.Option>
              <NomiSelect.Option value='sassy'>{t('nomi.settings.personaSassy')}</NomiSelect.Option>
            </NomiSelect>
          }
          footer={
            <Input.TextArea
              autoSize={{ minRows: 1, maxRows: 3 }}
              className='!bg-[var(--color-bg-1)] !border-[var(--color-border-2)] !rd-8px !px-10px !py-7px !leading-20px'
              placeholder={t('nomi.settings.personaCustomPlaceholder')}
              value={customDraft}
              onChange={onCustomChange}
            />
          }
        />
        <NomiSettingRow
          title={t('nomi.settings.quietHours')}
          description={t('nomi.settings.quietHoursHint')}
          controls={
            <TimePicker.RangePicker
              format='HH:mm'
              allowClear
              className='nomi-quiet-hours-picker !h-36px !w-260px shrink-0 !bg-[var(--color-bg-1)] !border-[var(--color-border-2)] !rd-8px max-[760px]:!w-full'
              value={
                profile.appearance.quiet_start && profile.appearance.quiet_end
                  ? [profile.appearance.quiet_start, profile.appearance.quiet_end]
                  : undefined
              }
              onChange={(value) => {
                const [quiet_start, quiet_end] = (value as string[] | undefined) ?? ['', ''];
                void patchCompanion({
                  appearance: { quiet_start: quiet_start || '', quiet_end: quiet_end || '' },
                });
              }}
            />
          }
        />
      </NomiSettingList>

      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.settings.deleteCompanion')}
          leading={
            <Attention
              theme='filled'
              size={14}
              fill='currentColor'
              className='line-height-0 shrink-0 text-[rgb(var(--danger-6))]'
            />
          }
          description={t('nomi.settings.deleteCompanionHint', { companionName })}
          controls={
            <Button status='danger' onClick={confirmDelete}>
              {t('nomi.settings.deleteCompanion')}
            </Button>
          }
        />
      </NomiSettingList>
    </div>
  );
};

export default SettingsTab;
