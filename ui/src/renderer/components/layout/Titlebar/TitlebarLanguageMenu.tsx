import React, { useCallback, useMemo } from 'react';
import { Dropdown, Menu } from '@arco-design/web-react';
import { Check, Translate } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import { changeLanguage, normalizeLanguageCode, supportedLanguages } from '@/renderer/services/i18n';

/** Native display names for each supported language (shown in the language's own script). */
const LANGUAGE_LABELS: Record<string, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
};

interface TitlebarLanguageMenuProps {
  /** Match the sibling titlebar icon buttons. */
  iconSize: number;
  strokeWidth?: number;
}

/**
 * Quick language switcher for the app titlebar.
 *
 * This is an *additional* fast-access entry — the canonical control still lives in
 * Settings > System > Language. Both call the same `changeLanguage()` pipeline
 * (reactive switch, backend persistence, tray/cross-window sync), so they stay in lockstep.
 */
const TitlebarLanguageMenu: React.FC<TitlebarLanguageMenuProps> = ({ iconSize, strokeWidth }) => {
  const { t, i18n } = useTranslation();
  const current = normalizeLanguageCode(i18n.language);

  const handleClickMenuItem = useCallback(
    (key: string) => {
      // No-op when picking the active language.
      if (normalizeLanguageCode(key) === normalizeLanguageCode(i18n.language)) return;

      const apply = () => {
        changeLanguage(key).catch((error: Error) => {
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
    [i18n.language]
  );

  const droplist = useMemo(
    () => (
      <Menu onClickMenuItem={handleClickMenuItem}>
        {supportedLanguages.map((lang) => {
          const active = normalizeLanguageCode(lang) === current;
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
    [handleClickMenuItem, current]
  );

  const label = t('settings.language');

  return (
    <InstantHoverTooltip content={label} position='bottom'>
      <Dropdown droplist={droplist} trigger='click' position='bl' getPopupContainer={() => document.body}>
        <button type='button' className='app-titlebar__button app-titlebar__button--nav' aria-label={label}>
          <Translate theme='outline' size={iconSize} fill='currentColor' strokeWidth={strokeWidth} />
        </button>
      </Dropdown>
    </InstantHoverTooltip>
  );
};

export default TitlebarLanguageMenu;
