/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useThemeContext } from '@/renderer/hooks/context/ThemeContext';
import { IconMoon, IconMoonFill, IconSun, IconSunFill } from '@arco-design/web-react/icon';
import { useTranslation } from 'react-i18next';

/** Compact light/dark control for the desktop appearance popover. */
export const ThemeSwitcher = () => {
  const { theme, setTheme } = useThemeContext();
  const { t } = useTranslation();
  const options = [
    { value: 'light' as const, label: t('settings.lightMode'), icon: IconSun, activeIcon: IconSunFill },
    { value: 'dark' as const, label: t('settings.darkMode'), icon: IconMoon, activeIcon: IconMoonFill },
  ];

  return (
    <div className='theme-switcher' role='radiogroup' aria-label={t('settings.theme')}>
      {options.map((option, index) => {
        const isActive = theme === option.value;
        const Icon = isActive ? option.activeIcon : option.icon;
        return (
          <button
            key={option.value}
            type='button'
            role='radio'
            aria-checked={isActive}
            tabIndex={isActive ? 0 : -1}
            className='theme-switcher__option'
            onClick={() => {
              if (!isActive) void setTheme(option.value);
            }}
            onKeyDown={(event) => {
              if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
              event.preventDefault();
              const nextIndex = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1 : (index + 1) % options.length;
              event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[nextIndex]?.focus();
              if (options[nextIndex].value !== theme) void setTheme(options[nextIndex].value);
            }}
          >
            <span className='theme-switcher__icon' aria-hidden='true'><Icon /></span>
            {option.label}
          </button>
        );
      })}
    </div>
  );
};
