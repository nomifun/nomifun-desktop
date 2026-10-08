/**
 * SkillMarketSettings — the skill market surface. A thin binding of the shared
 * MarketSettingsPanel to the skill ranking sources. "Add" calls the dedicated
 * Skill Library installer; it never creates or mutates an AgentSession.
 */
import { ipcBridge } from '@/common';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import { parseError } from '@/common/utils';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import MarketSettingsPanel from './MarketSettingsPanel';
import { ENHANCED_TOOLS_PAGE_STACK_CLASS } from './enhancedToolsLayout';
import {
  isSkillMarketItemInstalled,
  SKILL_MARKET_SOURCES,
} from './skill/skillMarket';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

const CACHE_KEY = 'nomifun.skillMarket.rankings.v4';
const AUTO_SYNC_KEY = 'nomifun.skillMarket.autoSynced.v4';
const INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';

type InstalledMarketIndex = Record<string, string[]>;

const readInstalledMarketIndex = (): InstalledMarketIndex => {
  try {
    const value = JSON.parse(localStorage.getItem(INSTALLED_MARKET_KEY) ?? '{}') as unknown;
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value)
        .filter(([id, names]) => id.length <= 180 && Array.isArray(names))
        .map(([id, names]) => [
          id,
          Array.from(
            new Set(
              names.filter((name: unknown): name is string => typeof name === 'string' && name.trim().length > 0)
            )
          ).slice(0, 32),
        ])
        .filter(([, names]) => names.length > 0)
    );
  } catch {
    return {};
  }
};

const writeInstalledMarketIndex = (index: InstalledMarketIndex): void => {
  try {
    localStorage.setItem(INSTALLED_MARKET_KEY, JSON.stringify(index));
  } catch {
    // Provenance is a UI optimization; the managed Skill Library is canonical.
  }
};

type SkillMarketSettingsProps = {
  active?: boolean;
};

const SkillMarketSettings: React.FC<SkillMarketSettingsProps> = ({ active = true }) => {
  const { t } = useTranslation();
  const [message, messageHolder] = useArcoMessage();
  const [installedSkillNames, setInstalledSkillNames] = useState<Set<string>>(new Set());
  const [installedMarketItemIds, setInstalledMarketItemIds] = useState<Set<string>>(new Set());
  const [installedStateLoading, setInstalledStateLoading] = useState(true);

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    setInstalledStateLoading(true);
    // Installation targets Nomi's managed Skill Library. External Agent
    // directories can be slow or unavailable and must not gate this flow.
    void ipcBridge.fs.listAvailableSkills
      .invoke()
      .then((skills) => {
        if (disposed) return;
        const names = new Set(skills.map((skill) => skill.name));
        const index = readInstalledMarketIndex();
        const reconciled = Object.fromEntries(
          Object.entries(index).filter(([, installedNames]) =>
            installedNames.every((installedName) => names.has(installedName))
          )
        );
        setInstalledSkillNames(names);
        setInstalledMarketItemIds(new Set(Object.keys(reconciled)));
        writeInstalledMarketIndex(reconciled);
      })
      .catch((error) => {
        // Keep Add available. The backend installer remains the authority and
        // will reject a real conflict without overwriting the existing skill.
        console.error('Failed to load Nomi skills for the market:', error);
      })
      .finally(() => {
        if (!disposed) setInstalledStateLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [active]);

  const isAdded = useCallback(
    (item: ISkillMarketItem) =>
      installedMarketItemIds.has(item.id) || isSkillMarketItemInstalled(item, installedSkillNames),
    [installedMarketItemIds, installedSkillNames]
  );

  const handleAdd = useCallback(
    async (item: ISkillMarketItem) => {
      try {
        const installed = await ipcBridge.fs.installSkillMarketItem.invoke({
          source: item.source,
          id: item.id,
          url: item.url,
        });
        setInstalledSkillNames((current) => {
          const next = new Set(current);
          for (const skillName of installed.skill_names) next.add(skillName);
          return next;
        });
        setInstalledMarketItemIds((current) => new Set(current).add(item.id));
        const index = readInstalledMarketIndex();
        index[item.id] = installed.skill_names;
        writeInstalledMarketIndex(index);
        message.success(
          t('settings.skillsMarket.installSuccess', {
            count: installed.skill_names.length,
            defaultValue: '技能已安装到 Nomi 技能库。',
          })
        );
      } catch (error) {
        console.error('Failed to install Skill Market item:', error);
        const reason = parseError(error).replace(/\s+/g, ' ').trim().slice(0, 240);
        message.error(
          reason
            ? t('settings.skillsMarket.installFailedWithReason', {
                reason,
                defaultValue: '技能安装失败：{{reason}}',
              })
            : t('settings.skillsMarket.installFailed', {
                defaultValue: '技能安装失败，请稍后重试。',
              })
        );
        throw error;
      }
    },
    [message, t]
  );

  return (
    <div className='flex flex-col h-full w-full'>
      {React.Children.toArray(messageHolder)}
      <div className={ENHANCED_TOOLS_PAGE_STACK_CLASS}>
        <MarketSettingsPanel
          title={t('settings.skillsMarket.title', { defaultValue: '技能市场' })}
          description={t('settings.skillsMarket.description', {
            defaultValue: '同步 ClawHub、LoopHub 与 SkillHub 最新榜单，并通过受控流程安装到 Nomi 技能库。',
          })}
          sources={SKILL_MARKET_SOURCES}
          cacheKey={CACHE_KEY}
          autoSyncKey={AUTO_SYNC_KEY}
          defaultSource='clawhub'
          searchPlaceholder={t('settings.skillsMarket.searchPlaceholder', { defaultValue: '搜索当前市场技能...' })}
          emptyText={t('settings.skillsMarket.empty', { defaultValue: '正在准备榜单，点击刷新可重新采集。' })}
          onAdd={handleAdd}
          showInstallCommand={false}
          isAdded={isAdded}
          addedStateLoading={installedStateLoading}
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
