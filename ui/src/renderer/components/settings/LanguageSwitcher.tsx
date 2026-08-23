import NomiSelect from '@/renderer/components/base/NomiSelect';
import type { SelectHandle } from '@arco-design/web-react/es/Select/interface';
import React, { useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import {
  SYSTEM_LANGUAGE,
  detectSystemLanguage,
  normalizeLanguageCode,
  setLanguagePreference,
} from '@/renderer/services/i18n';
import { useLanguagePreference } from '@/renderer/hooks/system/useLanguagePreference';

const LANGUAGE_LABELS: Record<string, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
};

const LanguageSwitcher: React.FC = () => {
  const { i18n, t } = useTranslation();
  const selectRef = useRef<SelectHandle>(null);
  const preference = useLanguagePreference(i18n.language);
  const systemLanguage = normalizeLanguageCode(detectSystemLanguage());
  const selectedValue = preference === SYSTEM_LANGUAGE ? SYSTEM_LANGUAGE : normalizeLanguageCode(preference);

  const handleLanguageChange = useCallback((value: string) => {
    // 切换前先 blur 触发元素，避免弹层和语言切换竞争布局
    // Blur before switching to avoid dropdown and language change fighting for layout
    selectRef.current?.blur?.();

    const applyLanguage = () => {
      const nextPreference = value === SYSTEM_LANGUAGE ? SYSTEM_LANGUAGE : normalizeLanguageCode(value);
      setLanguagePreference(nextPreference).catch((error: Error) => {
        console.error('Failed to change language:', error);
      });
    };

    if (typeof window !== 'undefined' && 'requestAnimationFrame' in window) {
      // 延迟到下一帧执行，确保 DOM 动画已完成 / defer to next frame so DOM animations finish
      window.requestAnimationFrame(() => window.requestAnimationFrame(applyLanguage));
    } else {
      setTimeout(applyLanguage, 0);
    }
  }, []);

  return (
    <div className='flex flex-col items-end gap-4px max-w-220px'>
      <NomiSelect ref={selectRef} className='w-200px' value={selectedValue} onChange={handleLanguageChange}>
        <NomiSelect.Option value={SYSTEM_LANGUAGE}>{t('settings.languageFollowSystem')}</NomiSelect.Option>
        <NomiSelect.Option value='zh-CN'>简体中文</NomiSelect.Option>
        <NomiSelect.Option value='en-US'>English</NomiSelect.Option>
      </NomiSelect>
      {preference === SYSTEM_LANGUAGE && (
        <span className='text-11px text-t-tertiary text-right leading-16px'>
          {t('settings.languageFollowSystemActive', { language: LANGUAGE_LABELS[systemLanguage] ?? systemLanguage })}
        </span>
      )}
    </div>
  );
};

export default LanguageSwitcher;
