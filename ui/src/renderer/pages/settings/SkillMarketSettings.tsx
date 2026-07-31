/**
 * SkillMarketSettings — the skill market surface. A thin binding of the shared
 * MarketSettingsPanel to the skill ranking sources: "Add" hands a reviewed,
 * never auto-sent installation draft to Nomi via the quick-start flow.
 */
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { resolveLocaleKey } from '@/common/utils';
import { useNomiQuickStart } from '@/renderer/hooks/agent/useNomiQuickStart';
import MarketSettingsPanel from './MarketSettingsPanel';
import {
  buildSkillMarketConversationName,
  buildSkillMarketInstallPrompt,
  SKILL_MARKET_SOURCES,
} from './skill/skillMarket';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';

const CACHE_KEY = 'nomifun.skillMarket.rankings.v4';
const AUTO_SYNC_KEY = 'nomifun.skillMarket.autoSynced.v4';

const SkillMarketSettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const localeKey = resolveLocaleKey(i18n.language);
  const { start } = useNomiQuickStart();

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
    <div className='flex flex-col h-full w-full'>
      <div className='space-y-16px pb-24px'>
        <MarketSettingsPanel
          title={t('settings.skillsMarket.title', { defaultValue: '技能市场' })}
          description={t('settings.skillsMarket.description', {
            defaultValue: '同步 ClawHub、LoopHub 与 SkillHub 最新榜单，选择技能后交给 Nomi 生成安装确认草稿。',
          })}
          sources={SKILL_MARKET_SOURCES}
          cacheKey={CACHE_KEY}
          autoSyncKey={AUTO_SYNC_KEY}
          defaultSource='clawhub'
          searchPlaceholder={t('settings.skillsMarket.searchPlaceholder', { defaultValue: '搜索当前市场技能...' })}
          emptyText={t('settings.skillsMarket.empty', { defaultValue: '正在准备榜单，点击刷新可重新采集。' })}
          onAdd={handleAdd}
          enableTagFilter
          testIdPrefix='skill-market'
          text={{
            syncSuccess: t('settings.skillsMarket.syncSuccess', { defaultValue: '技能市场已更新' }),
            syncKeptCache: t('settings.skillsMarket.syncKeptCache', { defaultValue: '未获取到新榜单，已保留本地缓存。' }),
            syncEmpty: t('settings.skillsMarket.syncEmpty', { defaultValue: '未采集到榜单数据。' }),
            syncError: t('settings.skillsMarket.syncError', { defaultValue: '更新技能市场失败' }),
            openFailed: t('settings.skillsMarket.openMarketFailed', { defaultValue: '无法打开技能市场' }),
            openInBrowser: t('settings.skillsMarket.openInBrowser', { defaultValue: '在浏览器中打开市场' }),
            noSearchMatch: (query, sourceLabel) =>
              t('settings.skillsMarket.noSearchMatch', {
                query,
                source: sourceLabel,
                defaultValue: `当前 ${sourceLabel} 未找到“${query}”相关技能。`,
              }),
            noFilterMatch: t('settings.skillsMarket.noMatch', { defaultValue: '没有符合当前筛选条件的技能。' }),
            lastUpdated: (time) =>
              t('settings.skillsMarket.lastUpdated', { time, defaultValue: '上次更新：{{time}}' }),
          }}
        />
      </div>
    </div>
  );
};

export default SkillMarketSettings;
