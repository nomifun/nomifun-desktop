// hooks/useTheme.ts
import { configService } from '@/common/config/configService';
import { broadcastThemeSync } from '@renderer/utils/theme/themeBroadcast';
import { useCallback, useLayoutEffect, useRef, useState } from 'react';

export type Theme = 'light' | 'dark';

const DEFAULT_THEME: Theme = 'light';
const THEME_CACHE_KEY = '__nomifun_theme';

const applyThemeToDom = (value: Theme) => {
  document.documentElement.setAttribute('data-theme', value);
  document.body.setAttribute('arco-theme', value);
};

const readCachedTheme = (): Theme => {
  try {
    const cached = localStorage.getItem(THEME_CACHE_KEY);
    if (cached === 'light' || cached === 'dark') return cached;
  } catch (_e) {
    /* noop */
  }
  return DEFAULT_THEME;
};

const readTheme = (config: typeof configService): Theme => {
  const stored = config.get('theme');
  if (stored === 'light' || stored === 'dark') return stored;
  // HTML applies the same hint before the bundle loads. Once settings are
  // available, an absent/invalid preference means the supported default.
  return stored === undefined && !config.isInitialized() ? readCachedTheme() : DEFAULT_THEME;
};

const applyTheme = (theme: Theme) => {
  applyThemeToDom(theme);
  try {
    localStorage.setItem(THEME_CACHE_KEY, theme);
  } catch (_e) {
    /* noop */
  }
};

const useTheme = (
  config = configService,
  broadcast = broadcastThemeSync
): [Theme, (theme: Theme) => Promise<void>] => {
  const [theme, setThemeState] = useState<Theme>(() => readTheme(config));
  const revision = useRef(0);

  useLayoutEffect(() => {
    let active = true;
    const sync = () => {
      const current = readTheme(config);
      setThemeState(current);
      applyTheme(current);
    };
    const unsubscribe = config.subscribe('theme', () => {
      revision.current++;
      sync();
    });
    sync();
    void config.whenReady().then(() => { if (active) sync(); });
    return () => {
      active = false;
      revision.current++;
      unsubscribe();
    };
  }, [config]);

  // Set theme with persistence
  const setTheme = useCallback(
    async (newTheme: Theme) => {
      const previous = readTheme(config);
      const writing = config.set('theme', newTheme);
      const current = ++revision.current;
      try {
        await writing;
        // 仅在持久化成功后广播：失败会走 catch 回滚，避免给独立窗口（桌宠）
        // 广播一个最终被回滚的值导致跨窗短暂不一致。
        if (revision.current === current && config.get('theme') === newTheme) broadcast(newTheme);
      } catch (error) {
        console.error('Failed to save theme:', error);
        if (revision.current === current && config.get('theme') === newTheme) {
          config.setLocal('theme', previous);
        }
        // Reconcile even an older failure: its predecessor may also have been
        // optimistic. ConfigService protects newer in-flight writes on reload.
        await config.reload();
      }
    },
    [config, broadcast]
  );

  return [theme, setTheme];
};

export default useTheme;
