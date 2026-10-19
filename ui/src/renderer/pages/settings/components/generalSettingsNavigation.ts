export type GeneralSettingsSection = 'preferences' | 'data';

const GENERAL_SETTINGS_FOCUS_IDS = ['preferences', 'conversation', 'notifications', 'files', 'storage', 'reset'] as const;
export type GeneralSettingsFocus = typeof GENERAL_SETTINGS_FOCUS_IDS[number];

/** Keep existing section links working after the settings categories are grouped. */
export function resolveGeneralSettingsSection(rawSection?: string | null): GeneralSettingsSection {
  return rawSection === 'files' || rawSection === 'data' ? 'data' : 'preferences';
}

export function resolveGeneralSettingsFocus(rawSection?: string | null, rawFocus?: string | null): GeneralSettingsFocus | undefined {
  const focus = GENERAL_SETTINGS_FOCUS_IDS.find((id) => id === rawFocus);
  if (focus) return focus;
  if (rawSection === 'conversation' || rawSection === 'notifications' || rawSection === 'files') return rawSection;
  return undefined;
}

/** Category navigation clears stale focus while retaining unrelated URL state. */
export function generalSettingsHref(section: GeneralSettingsSection, focus?: GeneralSettingsFocus, search?: string | URLSearchParams): string {
  const params = new URLSearchParams(search);
  params.set('section', section);
  params.delete('focus');
  if (focus) params.set('focus', focus);
  return `/settings/system?${params.toString()}`;
}
