import { ipcBridge } from '@/common';
import type { ISkillMarketItem, ISkillMarketPackageResponse } from '@/common/adapter/ipcBridge';
import type { CreatePresetRequest } from '@/common/types/agent/presetTypes';
import { parsePresetId, type PresetId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { Message } from '@arco-design/web-react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import { PRESET_MARKET_SOURCES } from '@/renderer/pages/settings/skill/skillMarket';

type PresetPackageMarketSettingsProps = {
  onImported: () => void | Promise<void>;
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

const PresetPackageMarketSettings: React.FC<PresetPackageMarketSettingsProps> = ({ onImported }) => {
  const { t, i18n } = useTranslation();

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
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

        const presetId = parsePresetId(uuidv7());
        const preset = buildPresetFromMarketPackage(
          { ...installedPackage.package, skill_slugs: installedPackage.installed_skill_names },
          i18n.language,
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
        Message.error(t('settings.presetMarket.addFailed', { defaultValue: 'Failed to add expert package.' }));
      }
    },
    [i18n.language, onImported, t]
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
    />
  );
};

export default PresetPackageMarketSettings;
