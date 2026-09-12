/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// hooks/useColorScheme.ts - Color Scheme Management Hook 配色方案管理
import { configService } from '@/common/config/configService';
import { useCallback, useLayoutEffect } from 'react';

// Supported color schemes 支持的配色方案类型
export type ColorScheme = 'default';

const DEFAULT_COLOR_SCHEME: ColorScheme = 'default';
const COLOR_SCHEME_CACHE_KEY = '__nomifun_colorScheme';

const applyColorScheme = (value: ColorScheme) => {
  document.documentElement.setAttribute('data-color-scheme', value);
  try {
    localStorage.setItem(COLOR_SCHEME_CACHE_KEY, value);
  } catch (_e) {
    /* noop */
  }
};

/**
 * Color scheme management hook 配色方案管理 Hook
 * @returns [colorScheme, setColorScheme] - Current color scheme and setter function 当前配色方案和设置函数
 */
const useColorScheme = (config = configService): [ColorScheme, (scheme: ColorScheme) => Promise<void>] => {
  useLayoutEffect(() => {
    let active = true;
    const sync = () => {
      // 'default' is the only supported scheme. Never apply retired wire values.
      if (active) applyColorScheme(DEFAULT_COLOR_SCHEME);
    };
    const unsubscribe = config.subscribe('colorScheme', sync);
    sync();
    void config.whenReady().then(sync);
    return () => {
      active = false;
      unsubscribe();
    };
  }, [config]);

  /**
   * Set color scheme with persistence 设置配色方案并持久化
   * Keep the write failure/reload behavior even though only one scheme remains.
   */
  const setColorScheme = useCallback(
    async (newScheme: ColorScheme) => {
      try {
        await config.set('colorScheme', newScheme);
      } catch (error) {
        console.error('Failed to save color scheme:', error);
        await config.reload();
      }
    },
    [config]
  );

  return [DEFAULT_COLOR_SCHEME, setColorScheme];
};

export default useColorScheme;
