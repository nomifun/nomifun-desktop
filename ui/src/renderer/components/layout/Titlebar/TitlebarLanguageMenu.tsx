import React, { useCallback, useMemo } from 'react';
import { Dropdown, Menu } from '@arco-design/web-react';
import { Check, Down } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import {
  SYSTEM_LANGUAGE,
  normalizeLanguageCode,
  setLanguagePreference,
  supportedLanguages,
  type LanguagePreference,
} from '@/renderer/services/i18n';
import { useLanguagePreference } from '@/renderer/hooks/system/useLanguagePreference';

/** Native display names for each supported language (shown in the language's own script). */
const LANGUAGE_LABELS: Record<string, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
};

interface TitlebarLanguageMenuProps {
  strokeWidth?: number;
}

/**
 * Quick language switcher for the app titlebar.
 *
 * This is an *additional* fast-access entry — the canonical control still lives in
 * Settings > System > Language. Both call the same `setLanguagePreference()` pipeline
 * (reactive switch, backend persistence, tray/cross-window sync), so they stay in lockstep.
 */
const TitlebarLanguageMenu: React.FC<TitlebarLanguageMenuProps> = ({ strokeWidth }) => {
  const { t, i18n } = useTranslation();
  const current = normalizeLanguageCode(i18n.language);
  const preference = useLanguagePreference(i18n.language);

  const handleClickMenuItem = useCallback(
    (key: string) => {
      const nextPreference: LanguagePreference =
        key === SYSTEM_LANGUAGE ? SYSTEM_LANGUAGE : normalizeLanguageCode(key);
      if (nextPreference === preference) return;

      const apply = () => {
        setLanguagePreference(nextPreference).catch((error: Error) => {
          console.error('Failed to change language:', error);
        });
      };

      // Defer to the next frame so the dropdown's close animation finishes before the
      // app-wide i18n re-render kicks in, avoiding a layout race (same guard as LanguageSwitcher).
      if (typeof window !== 'undefined' && 'requestAnimationFrame' in window) {
        window.requestAnimationFrame(() => window.requestAnimationFrame(apply));
      } else {
        apply();
      }
    },
    [preference]
  );

  const droplist = useMemo(
    () => (
      <Menu onClickMenuItem={handleClickMenuItem}>
        <Menu.Item key={SYSTEM_LANGUAGE}>
          <div className='flex items-start justify-between gap-12px min-w-220px'>
            <div className='min-w-0'>
              <div>{t('settings.languageFollowSystem')}</div>
              <div className='mt-2px max-w-220px text-11px leading-16px text-t-tertiary'>
                {t('settings.languageFollowSystemDesc', { language: LANGUAGE_LABELS[current] ?? current })}
              </div>
            </div>
            {preference === SYSTEM_LANGUAGE && <Check theme='outline' size={14} fill='currentColor' />}
          </div>
        </Menu.Item>
        {supportedLanguages.map((lang) => {
          const active = preference !== SYSTEM_LANGUAGE && normalizeLanguageCode(lang) === current;
          return (
            <Menu.Item key={lang}>
              <div className='flex items-center justify-between gap-12px min-w-120px'>
                <span>{LANGUAGE_LABELS[lang] ?? lang}</span>
                {active && <Check theme='outline' size={14} fill='currentColor' />}
              </div>
            </Menu.Item>
          );
        })}
      </Menu>
    ),
    [current, handleClickMenuItem, preference, t]
  );

  const currentLabel = current === 'zh-CN' ? '中文' : 'English';
  const tooltip = preference === SYSTEM_LANGUAGE
    ? t('settings.languageFollowSystemTooltip', { language: LANGUAGE_LABELS[current] ?? current })
    : t('settings.languageSwitchTooltip');

  return (
    <InstantHoverTooltip content={tooltip} position='bottom'>
      <Dropdown droplist={droplist} trigger='click' position='bl' getPopupContainer={() => document.body}>
        <button type='button' className='app-titlebar__language-button' aria-label={tooltip}>
          <span className='app-titlebar__language-name'>{currentLabel}</span>
          <Down theme='outline' size={14} fill='currentColor' strokeWidth={strokeWidth} />
        </button>
      </Dropdown>
    </InstantHoverTooltip>
  );
};

export default TitlebarLanguageMenu;
