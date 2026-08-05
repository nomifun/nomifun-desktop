/**
 * PluginSettingsPanel — installed-plugins list plus the ClawHub plugin market.
 * Market "Add" only prepares a DRAFT Nomi conversation (`send: false`); the
 * install command is never executed without the user reviewing and sending it.
 */
import { ipcBridge } from '@/common';
import type { IExtensionInfo, ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { resolveLocaleKey } from '@/common/utils';
import { Tag } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import MarketSettingsPanel from '@/renderer/pages/settings/MarketSettingsPanel';
import {
  buildSkillMarketConversationName,
  buildSkillMarketInstallPrompt,
  PLUGIN_MARKET_SOURCES,
} from '@/renderer/pages/settings/skill/skillMarket';
import { useNomiQuickStart } from '@/renderer/hooks/agent/useNomiQuickStart';

type PluginSettingsPanelProps = {
  section?: 'installed' | 'market' | 'both';
};

const PluginSettingsPanel: React.FC<PluginSettingsPanelProps> = ({ section = 'both' }) => {
  const { t, i18n } = useTranslation();
  const localeKey = resolveLocaleKey(i18n.language);
  const { start } = useNomiQuickStart();
  const [extensions, setExtensions] = useState<IExtensionInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const showInstalled = section !== 'market';
  const showMarket = section !== 'installed';

  useEffect(() => {
    if (!showInstalled) {
      setLoading(false);
      return;
    }

    void ipcBridge.extensions.getLoadedExtensions
      .invoke()
      .then(setExtensions)
      .catch((error) => {
        console.error('Failed to load installed plugins:', error);
        setExtensions([]);
      })
      .finally(() => setLoading(false));
  }, [showInstalled]);

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      await start({
        name: buildSkillMarketConversationName(item, localeKey),
        prompt: buildSkillMarketInstallPrompt(item, localeKey),
        send: false,
      });
    },
    [localeKey, start]
  );

  return (
    <div className='space-y-16px pb-24px'>
      {showInstalled && (
        <div className='bg-fill-2 rounded-24px p-20px'>
          <div className='flex items-start justify-between gap-12px mb-14px'>
            <div>
              <h2 className='m-0 text-22px font-600 text-t-primary'>
                {t('settings.plugins.installedTitle', { defaultValue: 'Installed Plugins' })}
              </h2>
              <p className='mt-6px mb-0 text-13px text-t-secondary'>
                {t('settings.plugins.installedDescription', {
                  defaultValue: 'Loaded Nomi extensions and plugin packages currently available to the app.',
                })}
              </p>
            </div>
          </div>

          {loading ? (
            <div className='py-24px text-center text-t-secondary text-14px'>
              {t('common.loading', { defaultValue: 'Loading...' })}
            </div>
          ) : extensions.length === 0 ? (
            <div className='py-24px text-center text-t-secondary text-14px border border-dashed border-arco-2 rd-12px'>
              {t('settings.plugins.emptyInstalled', { defaultValue: 'No installed plugins found.' })}
            </div>
          ) : (
            <div
              className='grid gap-12px'
              style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(260px, 100%), 1fr))' }}
            >
              {extensions.map((extension) => (
                <div
                  key={extension.name}
                  className='rounded-16px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] p-14px'
                >
                  <div className='flex items-start justify-between gap-10px'>
                    <div className='min-w-0'>
                      <div className='truncate text-14px font-medium text-t-primary'>
                        {extension.display_name || extension.name}
                      </div>
                      <div className='mt-3px text-11px text-t-tertiary font-mono truncate'>{extension.name}</div>
                    </div>
                    <Tag
                      size='small'
                      bordered={false}
                      className={
                        extension.enabled
                          ? '!bg-[rgba(var(--success-6),0.1)] !text-success-6'
                          : '!bg-[var(--color-fill-2)] !text-t-tertiary'
                      }
                    >
                      {extension.enabled
                        ? t('settings.plugins.stateEnabled', { defaultValue: 'Enabled' })
                        : t('settings.plugins.stateDisabled', { defaultValue: 'Disabled' })}
                    </Tag>
                  </div>
                  {extension.description && (
                    <div className='mt-10px text-12px leading-18px text-t-secondary line-clamp-2'>
                      {extension.description}
                    </div>
                  )}
                  <div className='mt-12px text-11px text-t-tertiary'>v{extension.version}</div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {showMarket && (
        <MarketSettingsPanel
          title={t('settings.plugins.marketTitle', { defaultValue: 'Plugin Market' })}
          description={t('settings.plugins.marketDescription', {
            defaultValue: 'Browse ClawHub plugins and prepare an installation draft for review.',
          })}
          sources={PLUGIN_MARKET_SOURCES}
          cacheKey='nomifun.pluginMarket.rankings.v1'
          autoSyncKey='nomifun.pluginMarket.autoSynced.v1'
          defaultSource='clawhub_plugins'
          searchPlaceholder={t('settings.plugins.searchPlaceholder', { defaultValue: 'Search plugins...' })}
          emptyText={t('settings.plugins.emptyMarket', { defaultValue: 'Refresh to load plugin market entries.' })}
          onAdd={handleAdd}
          testIdPrefix='plugin-market'
        />
      )}
    </div>
  );
};

export default PluginSettingsPanel;
