import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

import { configService } from '@/common/config/configService';
import { ipcBridge } from '@/common';
import { STORAGE_KEYS } from '@/common/config/storageKeys';
import i18nConfig from '@/common/config/i18n-config.json';
import {
  DEFAULT_LANGUAGE,
  SYSTEM_LANGUAGE,
  normalizeLanguageCode,
  mergeWithFallback,
  ensureAndSwitch,
  resolveLanguagePreference,
  type LanguageMode,
  type LanguagePreference,
  type LocaleData,
  type SupportedLanguage,
} from '@/common/config/i18n';

// Static imports for all locales to ensure packaged app can always switch language.
import enUS from './locales/en-US/index';
import zhCN from './locales/zh-CN/index';

export type { I18nKey, I18nModule } from './i18n-keys';

// Re-exports
export { normalizeLanguageCode, resolveLanguagePreference, SYSTEM_LANGUAGE } from '@/common/config/i18n';
export type { LanguageMode, LanguagePreference, SupportedLanguage } from '@/common/config/i18n';

export const supportedLanguages = i18nConfig.supportedLanguages;

const localeData: LocaleData = {
  'en-US': enUS,
  'zh-CN': zhCN,
};

const fallbackLocale = localeData[DEFAULT_LANGUAGE] ?? {};

// Cache for loaded translations
const loadedTranslations = new Map<string, Record<string, unknown>>();

// Pre-populate cache with the synchronously loaded fallback locale
loadedTranslations.set(DEFAULT_LANGUAGE, fallbackLocale as Record<string, unknown>);

function getLocaleModules(locale: string): Record<string, unknown> {
  const normalized = normalizeLanguageCode(locale);
  const modules = localeData[normalized] ?? fallbackLocale;
  if (normalized === DEFAULT_LANGUAGE) return modules;
  return mergeWithFallback(fallbackLocale, modules);
}

async function loadLocaleModules(locale: string): Promise<Record<string, unknown>> {
  const normalized = normalizeLanguageCode(locale);
  const cached = loadedTranslations.get(normalized);
  if (cached) return cached;

  const modules = getLocaleModules(normalized);
  loadedTranslations.set(normalized, modules);
  return modules;
}

const I18NEXT_LANGUAGE_STORAGE_KEY = 'i18nextLng';

function readStoredLanguageMode(): LanguageMode | undefined {
  if (typeof localStorage === 'undefined') return undefined;
  const stored = localStorage.getItem(STORAGE_KEYS.LANGUAGE_MODE);
  return stored === 'system' || stored === 'manual' ? stored : undefined;
}

function readStoredLanguageHint(): string | undefined {
  if (typeof localStorage === 'undefined') return undefined;
  return localStorage.getItem(I18NEXT_LANGUAGE_STORAGE_KEY) || undefined;
}

export function detectSystemLanguage(): SupportedLanguage {
  if (typeof navigator === 'undefined') return DEFAULT_LANGUAGE;

  const candidates = [
    navigator.language,
    ...(Array.isArray(navigator.languages) ? navigator.languages : []),
  ].filter((language): language is string => typeof language === 'string' && language.length > 0);

  return normalizeLanguageCode(candidates[0] || DEFAULT_LANGUAGE);
}

/**
 * Read the persisted preference, falling back to the previous localStorage hint
 * for offline/WebUI sessions and to the system locale for new installations.
 */
export function getLanguagePreference(): LanguagePreference {
  const configuredMode = configService.get('languageMode');
  const configuredLanguage = configService.get('language');

  if (configuredMode === 'system') return SYSTEM_LANGUAGE;
  if (configuredLanguage) return normalizeLanguageCode(configuredLanguage);

  const storedMode = readStoredLanguageMode();
  const storedLanguage = readStoredLanguageHint();
  if (storedMode === 'system') return SYSTEM_LANGUAGE;
  if (storedLanguage) return normalizeLanguageCode(storedLanguage);

  return SYSTEM_LANGUAGE;
}

export function getResolvedLanguagePreference(preference: LanguagePreference = getLanguagePreference()): SupportedLanguage {
  return resolveLanguagePreference(preference, detectSystemLanguage());
}

function syncLanguageStorage(preference: LanguagePreference, language: SupportedLanguage): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(I18NEXT_LANGUAGE_STORAGE_KEY, language);
  localStorage.setItem(
    STORAGE_KEYS.LANGUAGE_MODE,
    preference === SYSTEM_LANGUAGE ? 'system' : 'manual'
  );
}

// Initialize i18n with fallback locale loaded synchronously to avoid FOUC.
// NOTE: We intentionally do NOT use i18next-browser-languagedetector here.
// In WebUI mode the browser's localStorage is on a different origin than the
// Electron renderer, so the detector would read the wrong (or missing) value
// and fall back to navigator.language, causing a language mismatch (Issue #1176).
// Instead, we use localStorage only as a hint for the initial render and let
// configService (which bridges to the backend) be the single source of truth.
i18n
  .use(initReactI18next)
  .init({
    resources: {
      [DEFAULT_LANGUAGE]: {
        translation: fallbackLocale,
      },
    },
    lng: (() => {
      const storedMode = readStoredLanguageMode();
      const storedLanguage = readStoredLanguageHint();
      if (storedMode === 'system' || !storedLanguage) return detectSystemLanguage();
      return normalizeLanguageCode(storedLanguage);
    })(),
    fallbackLng: DEFAULT_LANGUAGE,
    debug: false,
    interpolation: { escapeValue: false },
  })
  .catch((error: Error) => {
    console.error('Failed to initialize i18n:', error);
  });

// Load initial language from configService (single source of truth).
// Wait until configService.whenReady() so we observe the authoritative value
// fetched from the backend rather than the empty cache that exists during
// module load.
async function initLanguage(): Promise<void> {
  try {
    await configService.whenReady();
    const preference = getLanguagePreference();
    const language = getResolvedLanguagePreference(preference);
    await ensureAndSwitch(i18n, language, loadLocaleModules);
    // Sync to localStorage so next page load can use it as a fast hint
    syncLanguageStorage(preference, language);
  } catch (error) {
    console.error('Failed to initialize language:', error);
  }
}

// Listen for language changes and lazy load translations
i18n.on('languageChanged', async (lang: string) => {
  const normalizedLang = normalizeLanguageCode(lang);
  if (i18n.hasResourceBundle(normalizedLang, 'translation')) return;

  try {
    const translation = await loadLocaleModules(normalizedLang);
    i18n.addResourceBundle(normalizedLang, 'translation', translation, true, true);
  } catch (error) {
    console.error(`Failed to load language ${normalizedLang}:`, error);
  }
});

// Initialize on module load
void initLanguage();

// Listen for language changes broadcast by the main process (from other renderers).
// This enables real-time sync between desktop and WebUI — when one changes language,
// the other updates immediately without requiring a restart.
ipcBridge.systemSettings.languageChanged.on(async ({ language }) => {
  const preference = getLanguagePreference();
  const normalized =
    preference === SYSTEM_LANGUAGE ? detectSystemLanguage() : normalizeLanguageCode(language);
  // Skip if already on this language (we're the one who triggered the change)
  if (i18n.language === normalized) return;
  await ensureAndSwitch(i18n, normalized, loadLocaleModules);
  syncLanguageStorage(preference, normalized);
});

/**
 * Persist and apply either a concrete language or the system language mode.
 */
export async function setLanguagePreference(preference: LanguagePreference): Promise<void> {
  const normalizedPreference =
    preference === SYSTEM_LANGUAGE ? SYSTEM_LANGUAGE : normalizeLanguageCode(preference);
  const normalized = getResolvedLanguagePreference(normalizedPreference);

  await ensureAndSwitch(i18n, normalized, loadLocaleModules);
  await configService.setBatch({
    language: normalized,
    languageMode: normalizedPreference === SYSTEM_LANGUAGE ? 'system' : 'manual',
  });
  syncLanguageStorage(normalizedPreference, normalized);
  // Notify main process to sync i18n (for tray menu, etc.)
  ipcBridge.systemSettings.changeLanguage.invoke({ language: normalized }).catch(() => {});
}

/** Change to a concrete language and mark the preference as manual. */
export async function changeLanguage(lang: string): Promise<void> {
  await setLanguagePreference(normalizeLanguageCode(lang));
}

/** Follow the language configured by the operating system. */
export async function useSystemLanguage(): Promise<void> {
  await setLanguagePreference(SYSTEM_LANGUAGE);
}

if (typeof window !== 'undefined') {
  window.addEventListener('languagechange', () => {
    if (getLanguagePreference() !== SYSTEM_LANGUAGE) return;
    useSystemLanguage().catch((error: Error) => {
      console.error('Failed to follow the system language:', error);
    });
  });
}
