/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ICompanionMood, ICompanionProfilePatch } from '@/common/adapter/ipcBridge';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import type { CompanionHandle } from './types';
import styles from './CompanionAppearanceQuickSettings.module.css';

interface Props {
  companion: CompanionHandle;
  disabled?: boolean;
  onSavingChange?: (saving: boolean) => void;
}

type Field = 'desktop' | 'mood';
const MOODS: ICompanionMood[] = ['content', 'happy', 'sleepy', 'worried', 'excited'];

const CompanionAppearanceQuickSettings: React.FC<Props> = ({ companion, disabled = false, onSavingChange }) => {
  const { t } = useTranslation();
  const { profile, status, patchCompanion } = companion;
  const [saving, setSaving] = useState<Field | null>(null);
  const [failed, setFailed] = useState<{ field: Field; patch: ICompanionProfilePatch } | null>(null);
  const [saved, setSaved] = useState(false);
  const savingRef = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  if (!profile) return null;

  const desktopAvailable = isTauriRuntime();
  const automaticMood = status?.automatic_mood ?? 'content';
  const moodName = t(`nomi.moods.${automaticMood}`, { defaultValue: automaticMood });
  const blocked = disabled || saving !== null;

  const save = async (field: Field, patch: ICompanionProfilePatch) => {
    if (disabled || savingRef.current || (field === 'desktop' && !desktopAvailable)) return;
    savingRef.current = true;
    setSaving(field);
    setFailed(null);
    setSaved(false);
    onSavingChange?.(true);
    try {
      const result = await patchCompanion(patch, { optimistic: false });
      if (!result) throw new Error('Companion is no longer available');
      if (mounted.current) setSaved(true);
    } catch {
      if (mounted.current) setFailed({ field, patch });
    } finally {
      savingRef.current = false;
      if (mounted.current) setSaving(null);
      onSavingChange?.(false);
    }
  };

  return (
    <div className={styles.settings}>
      <div className={styles.row}>
        <span className={styles.label}>{t('nomi.overview.deskVisible', { defaultValue: '桌面显示' })}</span>
        <NomiSelect size='mini' className={styles.select}
          aria-label={t('nomi.overview.deskVisible', { defaultValue: '桌面显示' })}
          value={profile.appearance.companion_enabled ? 'shown' : 'hidden'}
          disabled={blocked || !desktopAvailable} loading={saving === 'desktop'}
          onChange={(value: string) => void save('desktop', { appearance: { companion_enabled: value === 'shown' } })}>
          <NomiSelect.Option value='shown'>{t('nomi.overview.companionOn', { defaultValue: '已显示' })}</NomiSelect.Option>
          <NomiSelect.Option value='hidden'>{t('nomi.overview.companionOff', { defaultValue: '已隐藏' })}</NomiSelect.Option>
        </NomiSelect>
      </div>
      {!desktopAvailable && <span className={styles.hint}>{t('nomi.cohabit.quickSettings.desktopOnly', { defaultValue: '请在桌面端设置桌面形象' })}</span>}
      <div className={styles.row}>
        <span className={styles.label}>{t('nomi.cohabit.quickSettings.state', { defaultValue: '状态' })}</span>
        <NomiSelect size='mini' className={styles.select}
          aria-label={t('nomi.cohabit.quickSettings.state', { defaultValue: '状态' })}
          value={profile.mood_override ?? 'auto'} disabled={blocked || !status} loading={saving === 'mood'}
          onChange={(value: ICompanionMood | 'auto') => void save('mood', { mood_override: value === 'auto' ? null : value })}>
          <NomiSelect.Option value='auto'>{t('nomi.cohabit.quickSettings.automaticMood', { mood: moodName, defaultValue: '自动（{{mood}}）' })}</NomiSelect.Option>
          {MOODS.map((mood) => <NomiSelect.Option key={mood} value={mood}>{t(`nomi.moods.${mood}`)}</NomiSelect.Option>)}
        </NomiSelect>
      </div>
      <span className={styles.hint}>{profile.mood_override
        ? t('nomi.cohabit.quickSettings.manualMoodHint', { defaultValue: '手动保持，切回“自动”时恢复' })
        : t('nomi.cohabit.quickSettings.automaticMoodHint', { defaultValue: '自动跟随伙伴情绪' })}</span>
      {saving && <span className={styles.hint} role='status'>{t('nomi.cohabit.quickSettingsSaving', { defaultValue: '保存中…' })}</span>}
      {failed && <div className={styles.error} role='alert'>
        <span>{t('nomi.cohabit.quickSettings.saveFailed', { defaultValue: '保存失败，已保留原值。' })}</span>
        <button type='button' disabled={blocked} onClick={() => void save(failed.field, failed.patch)}>{t('agentSettings.actions.retry', { defaultValue: '重试' })}</button>
      </div>}
      {saved && !saving && !failed && <span className={styles.hint} role='status'>{t('nomi.cohabit.quickSettings.saved', { defaultValue: '已保存' })}</span>}
    </div>
  );
};

export default CompanionAppearanceQuickSettings;
