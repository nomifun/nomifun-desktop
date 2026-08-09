/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Checkbox, Message, Popconfirm, Tag } from '@arco-design/web-react';
import { Plus } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import type { ChannelPluginId, CsAgentId } from '@/common/types/ids';
import NomiModal from '@/renderer/components/base/NomiModal';
import { CHANNEL_PLATFORMS, PlatformConfigBody } from '@/renderer/components/channels/PlatformConfigBody';
import {
  retargetConfigAfterStatus,
  statusInOwnerDomain,
  type ChannelConfigTarget,
} from '@/renderer/components/channels/channelStatusSelection';
import type { ChannelPlatform } from '@/renderer/components/settings/SettingsModal/contents/channels/channelTarget';
import { csBotBindingState, findNewlyCreatedCsBot, selectCsChannelBots } from './csChannelBots';

/**
 * 客服渠道机器人自闭环管理区（渠道所有权分域）：
 * - 列表只显示 `owner_domain === 'customer_service'` 的 bot（绑本客服 / 绑其他
 *   客服 / 未绑定），伙伴域 bot 永不入池；
 * - 页内「新建渠道机器人」复用共享的 PlatformConfigBody 配置面，创建请求打
 *   `owner_domain='customer_service'` 且绝不携带 companion 绑定；
 * - 创建成功（快照差分探测到新行）即自动 replaceBindings 纳入本客服。
 */
const CsChannelBotsSection: React.FC<{ csAgentId: CsAgentId }> = ({ csAgentId }) => {
  const { t } = useTranslation();

  const [statuses, setStatuses] = useState<IChannelPluginStatus[]>([]);
  const [ownerByBot, setOwnerByBot] = useState<ReadonlyMap<ChannelPluginId, CsAgentId>>(new Map());
  const [agentNameById, setAgentNameById] = useState<ReadonlyMap<CsAgentId, string>>(new Map());
  const [savingBindings, setSavingBindings] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  // Config modal target: with channelPluginId = edit; without = create mode.
  const [configTarget, setConfigTarget] = useState<ChannelConfigTarget>(null);
  // 创建弹窗打开时的全量快照：之后新出现的 cs 域 bot 即"本弹窗创建的那一行"。
  const knownIdsRef = useRef<ReadonlySet<ChannelPluginId>>(new Set());
  // 防止同一新建 bot 被重复自动绑定。
  const autoBoundRef = useRef<Set<ChannelPluginId>>(new Set());

  const refreshAll = useCallback(async () => {
    try {
      const [plugins, agents] = await Promise.all([
        ipcBridge.channel.getPluginStatus.invoke(),
        ipcBridge.customerService.listAgents.invoke(),
      ]);
      setStatuses(plugins ?? []);
      const agentList = agents ?? [];
      setAgentNameById(new Map(agentList.map((agent) => [agent.cs_agent_id, agent.name])));
      // 全量绑定归属（bot → 客服）：区分「绑本客服 / 绑其他客服 / 未绑定」。
      const bindingLists = await Promise.all(
        agentList.map((agent) =>
          ipcBridge.customerService.listBindings
            .invoke({ cs_agent_id: agent.cs_agent_id })
            .catch(() => [])
        )
      );
      const owners = new Map<ChannelPluginId, CsAgentId>();
      for (const bindings of bindingLists) {
        for (const binding of bindings ?? []) {
          owners.set(binding.channel_plugin_id, binding.cs_agent_id);
        }
      }
      setOwnerByBot(owners);
    } catch (error) {
      console.error('[CsChannelBots] Failed to load channel bots:', error);
    }
  }, []);

  useEffect(() => {
    void refreshAll();
    const unsubscribe = ipcBridge.channel.pluginStatusChanged.on(() => void refreshAll());
    return () => unsubscribe();
  }, [refreshAll]);

  const csBots = useMemo(() => selectCsChannelBots(statuses), [statuses]);
  const boundIds = useMemo(
    () =>
      csBots
        .filter((bot) => ownerByBot.get(bot.plugin_id) === csAgentId)
        .map((bot) => bot.plugin_id),
    [csBots, ownerByBot, csAgentId]
  );

  const saveBindings = useCallback(
    async (next: ChannelPluginId[], successMessage?: string) => {
      setSavingBindings(true);
      try {
        await ipcBridge.customerService.replaceBindings.invoke({
          cs_agent_id: csAgentId,
          channel_plugin_ids: next,
        });
        Message.success(
          successMessage ?? t('customerService.bindings.saved', { defaultValue: '绑定已更新' })
        );
      } catch (error) {
        Message.error(error instanceof Error ? error.message : String(error));
      } finally {
        setSavingBindings(false);
      }
      await refreshAll();
    },
    [csAgentId, refreshAll, t]
  );

  // 创建弹窗内新建成功 → 采纳该行（弹窗转为编辑该实体）并自动绑定到本客服。
  useEffect(() => {
    if (!configTarget || configTarget.channelPluginId) return;
    const created = findNewlyCreatedCsBot(statuses, configTarget.platform, knownIdsRef.current);
    if (!created) return;
    setConfigTarget((prev) => retargetConfigAfterStatus(prev, created));
    if (!autoBoundRef.current.has(created.plugin_id)) {
      autoBoundRef.current.add(created.plugin_id);
      void saveBindings(
        [...boundIds, created.plugin_id],
        t('customerService.bindings.autoBound', { defaultValue: '已创建并自动绑定到本客服' })
      );
    }
  }, [statuses, configTarget, boundIds, saveBindings, t]);

  const startCreate = (platform: ChannelPlatform) => {
    knownIdsRef.current = new Set(statuses.map((status) => status.plugin_id));
    setPickerOpen(false);
    setConfigTarget({ platform });
  };

  const deleteBot = async (bot: IChannelPluginStatus) => {
    try {
      await ipcBridge.channel.deletePlugin.invoke({ plugin_id: bot.plugin_id });
      await refreshAll();
    } catch (error) {
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const configPlatformMeta = useMemo(
    () => CHANNEL_PLATFORMS.find((platform) => platform.id === configTarget?.platform),
    [configTarget?.platform]
  );

  const statusTag = (bot: IChannelPluginStatus) => {
    if (!bot.hasToken) {
      return (
        <Tag size='small' color='gray' className='shrink-0'>
          {t('nomi.settings.remoteStatusNotConfigured')}
        </Tag>
      );
    }
    if (bot.enabled && bot.connected) {
      return (
        <Tag size='small' color='green' className='shrink-0'>
          {t('nomi.settings.remoteStatusRunning')}
        </Tag>
      );
    }
    if (bot.enabled) {
      return (
        <Tag size='small' bordered={false} className='shrink-0 !bg-primary-1 !text-primary-6'>
          {t('nomi.settings.remoteStatusEnabled')}
        </Tag>
      );
    }
    return (
      <Tag size='small' color='gray' className='shrink-0'>
        {t('nomi.settings.remoteStatusDisabled')}
      </Tag>
    );
  };

  return (
    <div className='flex min-w-0 flex-col gap-8px'>
      <div className='flex min-w-0 flex-wrap items-start justify-between gap-10px 12px'>
        <span className='min-w-[220px] flex-1 text-12px leading-18px text-t-tertiary'>
          {t('customerService.bindings.domainHint', {
            defaultValue: '客服使用自己的渠道机器人，与桌面伙伴的渠道相互独立。',
          })}
        </span>
        <Button size='small' type='primary' className='shrink-0' onClick={() => setPickerOpen(true)}>
          <span className='inline-flex items-center gap-4px'>
            <Plus theme='outline' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            {t('customerService.bindings.createBot', { defaultValue: '新建渠道机器人' })}
          </span>
        </Button>
      </div>

      {csBots.length === 0 ? (
        <div className='break-words text-13px text-t-tertiary'>
          {t('customerService.bindings.noBots', {
            defaultValue: '还没有客服渠道机器人 —— 在这里创建一个，创建成功后会自动绑定到本客服。',
          })}
        </div>
      ) : (
        <div className='flex min-w-0 flex-col gap-8px'>
          {csBots.map((bot) => {
            const binding = csBotBindingState(bot.plugin_id, csAgentId, ownerByBot);
            const platform = CHANNEL_PLATFORMS.find((p) => p.id === bot.type);
            return (
              <div key={bot.plugin_id} className='flex min-w-0 flex-wrap items-center gap-10px text-13px text-t-primary'>
                <Checkbox
                  checked={binding.kind === 'boundToThis'}
                  disabled={savingBindings}
                  onChange={(checked: boolean) => {
                    const next = checked
                      ? [...boundIds, bot.plugin_id]
                      : boundIds.filter((id) => id !== bot.plugin_id);
                    void saveBindings(next);
                  }}
                />
                <span className='min-w-[120px] flex-1 basis-[160px] truncate'>{bot.name}</span>
                <Tag size='small' className='shrink-0'>
                  {bot.type}
                </Tag>
                {statusTag(bot)}
                {binding.kind === 'boundToOther' && (
                  <Tag size='small' color='orange' className='shrink-0'>
                    {t('customerService.bindings.boundToOther', {
                      defaultValue: '已绑客服：{{name}}',
                      name: agentNameById.get(binding.csAgentId) ?? binding.csAgentId,
                    })}
                  </Tag>
                )}
                {binding.kind === 'unbound' && (
                  <Tag size='small' color='gray' className='shrink-0'>
                    {t('customerService.bindings.unbound', { defaultValue: '未绑定' })}
                  </Tag>
                )}
                <div className='ml-auto flex shrink-0 flex-wrap items-center justify-end gap-8px'>
                  {platform && (
                    <Button
                      size='mini'
                      onClick={() => setConfigTarget({ platform: platform.id, channelPluginId: bot.plugin_id })}
                    >
                      {t('nomi.settings.remoteConfigure')}
                    </Button>
                  )}
                  <Popconfirm
                    title={t('nomi.settings.remoteDeleteConfirm')}
                    okButtonProps={{ status: 'danger' }}
                    onOk={() => void deleteBot(bot)}
                  >
                    <Button size='mini' status='danger'>
                      {t('nomi.settings.remoteDeleteBot')}
                    </Button>
                  </Popconfirm>
                </div>
              </div>
            );
          })}
          <div className='text-12px text-t-quaternary'>
            {t('customerService.bindings.hint', {
              defaultValue: '绑定后，该机器人的所有来访消息都会交给这位客服接待（陌生访客免配对码）。',
            })}
          </div>
        </div>
      )}

      {/* 新建入口：先挑平台，再进共享配置面（create mode） */}
      <NomiModal
        visible={pickerOpen}
        onCancel={() => setPickerOpen(false)}
        header={{
          title: t('customerService.bindings.createTitle', { defaultValue: '新建客服渠道机器人' }),
          showClose: true,
        }}
        footer={null}
        style={{ width: 'min(520px, calc(100vw - 32px))' }}
      >
        <div className='flex flex-col gap-8px py-8px'>
          <div className='text-12px text-t-tertiary'>
            {t('customerService.bindings.pickPlatform', { defaultValue: '选择渠道平台' })}
          </div>
          <div className='grid grid-cols-2 gap-8px'>
            {CHANNEL_PLATFORMS.map(({ id, logo, titleKey, fallback }) => (
              <Button key={id} className='!h-40px' onClick={() => startCreate(id)}>
                <span className='inline-flex items-center gap-8px'>
                  <img src={logo} alt='' className='w-16px h-16px object-contain' />
                  {t(titleKey, fallback)}
                </span>
              </Button>
            ))}
          </div>
        </div>
      </NomiModal>

      {/* 渠道配置（创建/编辑）：寻址客服域，绝不携带 companion 绑定 */}
      <NomiModal
        visible={Boolean(configTarget)}
        onCancel={() => {
          setConfigTarget(null);
          void refreshAll();
        }}
        header={{
          title: t('nomi.settings.remoteConfigTitle', {
            channel: configPlatformMeta ? t(configPlatformMeta.titleKey, configPlatformMeta.fallback) : '',
          }),
          showClose: true,
        }}
        footer={null}
        style={{ width: 'min(720px, calc(100vw - 32px))' }}
        contentStyle={{ maxHeight: 'calc(80vh - 80px)', padding: '0 2px' }}
      >
        {configTarget && (
          <PlatformConfigBody
            key={configTarget.channelPluginId ?? `${configTarget.platform}:new`}
            platform={configTarget.platform}
            status={
              configTarget.channelPluginId
                ? (statuses.find((s) => s.plugin_id === configTarget.channelPluginId) ?? null)
                : null
            }
            channelTarget={{
              channelPluginId: configTarget.channelPluginId,
              ownerDomain: 'customer_service',
            }}
            onStatusChange={(status) => {
              // 只采纳客服域行：表单的 create-mode 解析是启发式的，别让
              // 伙伴域 bot 把弹窗重定向到错误实体（自动绑定走快照差分）。
              if (status && statusInOwnerDomain(status, 'customer_service')) {
                setStatuses((prev) => [
                  ...prev.filter((s) => s.plugin_id !== status.plugin_id),
                  status,
                ]);
                setConfigTarget((prev) => retargetConfigAfterStatus(prev, status));
              }
              void refreshAll();
            }}
            refreshStatuses={refreshAll}
          />
        )}
      </NomiModal>
    </div>
  );
};

export default CsChannelBotsSection;
