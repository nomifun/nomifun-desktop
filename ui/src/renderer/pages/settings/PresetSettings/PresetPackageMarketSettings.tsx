import { ipcBridge } from '@/common';
import type { ISkillMarketItem, ISkillMarketPackageResponse } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { CreatePresetRequest, Preset } from '@/common/types/agent/presetTypes';
import { parsePresetId, type PresetId } from '@/common/types/ids';
import { resolveLocaleKey, uuidv7 } from '@/common/utils';
import { Message } from '@arco-design/web-react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import { PRESET_MARKET_SOURCES } from '@/renderer/pages/settings/skill/skillMarket';

type PresetPackageMarketSettingsProps = {
  onImported: () => void | Promise<void>;
  presets: Preset[];
  addedStateLoading?: boolean;
};

const PRESET_MARKET_ID_STORAGE_KEY = 'nomifun.presetMarket.itemPresetIds.v1';

type KeyValueStorage = Pick<Storage, 'getItem' | 'setItem'>;

const marketPresetStorage = (): KeyValueStorage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export const readPresetMarketIds = (storage: KeyValueStorage | null): Record<string, PresetId> => {
  if (!storage) return {};
  try {
    const parsed = JSON.parse(storage.getItem(PRESET_MARKET_ID_STORAGE_KEY) || '{}') as Record<string, unknown>;
    return Object.fromEntries(
      Object.entries(parsed).flatMap(([itemId, value]) => {
        try {
          return [[itemId, parsePresetId(value)]];
        } catch {
          return [];
        }
      })
    );
  } catch {
    return {};
  }
};

export const getOrCreatePresetMarketId = (
  itemId: string,
  storage: KeyValueStorage | null = marketPresetStorage()
): PresetId => {
  const ids = readPresetMarketIds(storage);
  const existing = ids[itemId];
  if (existing) return existing;
  const presetId = parsePresetId(uuidv7());
  try {
    storage?.setItem(PRESET_MARKET_ID_STORAGE_KEY, JSON.stringify({ ...ids, [itemId]: presetId }));
  } catch {
    // The stable ID is an extra duplicate guard; installed-content matching
    // below still protects the UI when browser storage is unavailable.
  }
  return presetId;
};

const normalizePackageText = (value: string | undefined): string => (value ?? '').trim().toLocaleLowerCase();

export const isPresetMarketItemInstalled = (
  item: Pick<ISkillMarketItem, 'id' | 'name' | 'description'>,
  presets: readonly Preset[],
  storage: KeyValueStorage | null = marketPresetStorage()
): boolean => {
  const mappedPresetId = readPresetMarketIds(storage)[item.id];
  if (mappedPresetId && presets.some((preset) => preset.preset_id === mappedPresetId)) return true;

  // Compatibility for expert packages installed before stable market IDs
  // were recorded. Pair name and description to avoid blocking an unrelated
  // user preset that merely has the same title.
  const itemName = normalizePackageText(item.name);
  const itemDescription = normalizePackageText(item.description);
  return presets.some(
    (preset) =>
      preset.source === 'user' &&
      normalizePackageText(preset.name) === itemName &&
      normalizePackageText(preset.description) === itemDescription
  );
};

const PACKAGE_METADATA_FIELDS = new Set([
  'aliases',
  'author',
  'children',
  'compatibility',
  'description',
  'display_name',
  'metadata',
  'name',
  'orchestration',
  'package_type',
  'version',
]);
const SKILL_SLUG = /^[A-Za-z0-9_.-]{1,96}$/;
const MAX_INSTALLED_SKILL_NAME_LENGTH = 160;
const UNSAFE_INSTALLED_SKILL_NAME = /[\/\\]|\.\.|[\x00-\x1F\x7F]/;

type SkillBindingSource = 'market' | 'installed';
export type PresetPackageAddErrorKind = 'timeout' | 'upstream' | 'not_found' | 'generic';

export const classifyPresetPackageAddError = (error: unknown): PresetPackageAddErrorKind => {
  if (!isBackendHttpError(error)) return 'generic';
  if (error.code === 'TIMEOUT') return 'timeout';
  if (error.code === 'BAD_GATEWAY') return 'upstream';
  if (error.code === 'NOT_FOUND') return 'not_found';
  return 'generic';
};

const isSkillBindingName = (skillName: string, source: SkillBindingSource) =>
  source === 'market'
    ? SKILL_SLUG.test(skillName)
    : skillName.length > 0 &&
      skillName.length <= MAX_INSTALLED_SKILL_NAME_LENGTH &&
      !UNSAFE_INSTALLED_SKILL_NAME.test(skillName);

const packageSkillBindings = (
  skillSlugs: string[],
  source: SkillBindingSource = 'market'
): CreatePresetRequest['included_skills'] => {
  const seen = new Set<string>();
  return skillSlugs
    .map((skillName) => skillName.trim())
    .filter(
      (skillName) => isSkillBindingName(skillName, source) && !PACKAGE_METADATA_FIELDS.has(skillName.toLowerCase())
    )
    .filter((skillName) => {
      const key = skillName.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((skill_name) => ({ skill_name, required: false }));
};

export const buildPresetFromMarketPackage = (
  resolved: ISkillMarketPackageResponse,
  localeKey: string,
  presetId?: PresetId,
  skillBindingSource: SkillBindingSource = 'market'
): CreatePresetRequest => ({
  preset_id: presetId,
  name: resolved.name,
  description: resolved.description,
  instructions: resolved.instructions,
  instructions_i18n: { [localeKey || 'zh-CN']: resolved.instructions },
  avatar: resolved.avatar,
  targets: ['conversation', 'execution_step'],
  included_skills: packageSkillBindings(resolved.skill_slugs, skillBindingSource),
});

const PresetPackageMarketSettings: React.FC<PresetPackageMarketSettingsProps> = ({
  onImported,
  presets,
  addedStateLoading = false,
}) => {
  const { t, i18n } = useTranslation();

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      // Reserve the durable market→preset identity before the network step so
      // two app windows cannot mint separate preset IDs for the same package.
      const presetId = getOrCreatePresetMarketId(item.id);
      try {
        const installedPackage = await ipcBridge.fs.installSkillMarketPackage.invoke({
          source: item.source,
          id: item.id,
          url: item.url,
        });
        const failedSkillCount = installedPackage.errors?.length ?? 0;
        const failedSkillNames = (installedPackage.errors ?? [])
          .slice(0, 3)
          .map((error) => error.skill_slug)
          .join(', ');
        if (installedPackage.installed_skill_names.length === 0 && failedSkillCount > 0) {
          console.error('Failed to install expert package skills:', installedPackage.errors);
          Message.error(
            t('settings.presetMarket.skillInstallFailed', {
              count: failedSkillCount,
              skills: failedSkillNames,
              defaultValue: 'Failed to install {{count}} expert package skill(s): {{skills}}.',
            })
          );
          return;
        }

        const preset = buildPresetFromMarketPackage(
          { ...installedPackage.package, skill_slugs: installedPackage.installed_skill_names },
          resolveLocaleKey(i18n.language),
          presetId,
          'installed'
        );
        const result = await ipcBridge.presets.import.invoke({ presets: [preset] });
        if (result.imported > 0) {
          let stateUpdateFailed = false;
          try {
            await ipcBridge.presets.setState.invoke({
              preset_id: presetId,
              auto_selectable: true,
            });
          } catch (error) {
            stateUpdateFailed = true;
            console.error('Failed to enable expert package auto-selection:', error);
          }
          if (failedSkillCount > 0) {
            console.warn('Some expert package skills failed to install:', installedPackage.errors);
            Message.warning(
              t('settings.presetMarket.partialSkillInstall', {
                count: failedSkillCount,
                skills: failedSkillNames,
                defaultValue: 'Expert package added, but {{count}} skill(s) failed to install: {{skills}}.',
              })
            );
          } else if (!stateUpdateFailed) {
            Message.success(t('settings.presetMarket.addSuccess', { defaultValue: 'Expert package and skills added.' }));
          }
          if (stateUpdateFailed) {
            Message.warning(
              t('settings.presetMarket.stateUpdateFailed', {
                defaultValue: 'Expert package added, but automatic selection could not be enabled.',
              })
            );
          }
          try {
            await onImported();
          } catch (error) {
            console.error('Failed to refresh imported expert package list:', error);
          }
        } else if (result.skipped > 0) {
          Message.warning(t('settings.presetMarket.addSkipped', { defaultValue: 'Expert package already exists.' }));
        } else {
          Message.error(result.errors[0]?.error || t('settings.presetMarket.addFailed', { defaultValue: 'Failed to add expert package.' }));
        }
      } catch (error) {
        console.error('Failed to add expert package:', error);
        const errorKind = classifyPresetPackageAddError(error);
        const errorMessage = {
          timeout: t('settings.presetMarket.timeout', {
            defaultValue: 'SkillHub took too long to respond. Please try again.',
          }),
          upstream: t('settings.presetMarket.upstreamUnavailable', {
            defaultValue: 'SkillHub is temporarily unavailable. Please try again later.',
          }),
          not_found: t('settings.presetMarket.notFound', {
            defaultValue: 'This expert package is no longer available. Refresh the market and try again.',
          }),
          generic: t('settings.presetMarket.addFailed', { defaultValue: 'Failed to add expert package.' }),
        }[errorKind];
        Message.error(errorMessage);
      }
    },
    [i18n.language, onImported, t]
  );

  const isAdded = useCallback(
    (item: ISkillMarketItem) => isPresetMarketItemInstalled(item, presets),
    [presets]
  );

  return (
    <MarketSettingsPanel
      title={t('settings.presetMarket.title', { defaultValue: 'Preset Market' })}
      description={t('settings.presetMarket.description', {
        defaultValue: 'Browse SkillHub expert packages and add them as reusable Nomi presets.',
      })}
      sources={PRESET_MARKET_SOURCES}
      cacheKey='nomifun.presetMarket.rankings.v1'
      autoSyncKey='nomifun.presetMarket.autoSynced.v1'
      defaultSource='skillhub_packages'
      searchPlaceholder={t('settings.presetMarket.searchPlaceholder', { defaultValue: 'Search expert packages...' })}
      emptyText={t('settings.presetMarket.empty', { defaultValue: 'Refresh to load expert packages.' })}
      onAdd={handleAdd}
      isAdded={isAdded}
      addedStateLoading={addedStateLoading}
      testIdPrefix='preset-market'
    />
  );
};

export default PresetPackageMarketSettings;
