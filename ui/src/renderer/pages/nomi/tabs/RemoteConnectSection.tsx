/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel, webui, type IWebUIStatus } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import NomiModal from '@/renderer/components/base/NomiModal';
import ChannelDingTalkLogo from '@/renderer/assets/channel-logos/dingtalk.svg';
import ChannelDiscordLogo from '@/renderer/assets/channel-logos/discord.svg';
import ChannelLarkLogo from '@/renderer/assets/channel-logos/lark.svg';
import ChannelMatrixLogo from '@/renderer/assets/channel-logos/matrix.svg';
import ChannelMattermostLogo from '@/renderer/assets/channel-logos/mattermost.svg';
import ChannelNostrLogo from '@/renderer/assets/channel-logos/nostr.svg';
import ChannelQQBotLogo from '@/renderer/assets/channel-logos/qqbot.svg';
import ChannelSlackLogo from '@/renderer/assets/channel-logos/slack.svg';
import ChannelTelegramLogo from '@/renderer/assets/channel-logos/telegram.svg';
import ChannelTwitchLogo from '@/renderer/assets/channel-logos/twitch.svg';
import ChannelWecomLogo from '@/renderer/assets/channel-logos/wecom.svg';
import ChannelWeixinLogo from '@/renderer/assets/channel-logos/weixin.svg';
import type { ChannelTarget, MasterAgentPlatform } from '@/renderer/components/settings/SettingsModal/contents/channels/channelTarget';
import DingTalkConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/DingTalkConfigForm';
import DiscordConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/DiscordConfigForm';
import LarkConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/LarkConfigForm';
import MatrixConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/MatrixConfigForm';
import MattermostConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/MattermostConfigForm';
import NostrConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/NostrConfigForm';
import QQBotConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/QQBotConfigForm';
import SlackConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/SlackConfigForm';
import TwitchConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/TwitchConfigForm';
import TelegramConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/TelegramConfigForm';
import WecomConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/WecomConfigForm';
import WeixinConfigForm from '@/renderer/components/settings/SettingsModal/contents/channels/WeixinConfigForm';
import { Button, Message, Modal, Switch, Tag } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useCompanions } from '../useNomi';

/** Builtin IM platforms a companion can greet (same set as channel master-agent mode). */
const PLATFORMS: ReadonlyArray<{ id: MasterAgentPlatform; logo: string; titleKey: string; fallback: string }> = [
  { id: 'weixin', logo: ChannelWeixinLogo, titleKey: 'settings.channels.weixinTitle', fallback: 'WeChat' },
  { id: 'lark', logo: ChannelLarkLogo, titleKey: 'settings.channels.larkTitle', fallback: 'Lark / Feishu' },
  { id: 'wecom', logo: ChannelWecomLogo, titleKey: 'settings.channels.wecomTitle', fallback: 'WeCom' },
  { id: 'dingtalk', logo: ChannelDingTalkLogo, titleKey: 'settings.channels.dingtalkTitle', fallback: 'DingTalk' },
  { id: 'qqbot', logo: ChannelQQBotLogo, titleKey: 'settings.channels.qqbotTitle', fallback: 'QQ Bot' },
  { id: 'telegram', logo: ChannelTelegramLogo, titleKey: 'settings.channels.telegramTitle', fallback: 'Telegram' },
  { id: 'discord', logo: ChannelDiscordLogo, titleKey: 'settings.channels.discordTitle', fallback: 'Discord' },
  { id: 'slack', logo: ChannelSlackLogo, titleKey: 'settings.channels.slackTitle', fallback: 'Slack' },
  { id: 'matrix', logo: ChannelMatrixLogo, titleKey: 'settings.channels.matrixTitle', fallback: 'Matrix' },
  { id: 'mattermost', logo: ChannelMattermostLogo, titleKey: 'settings.channels.mattermostTitle', fallback: 'Mattermost' },
  { id: 'twitch', logo: ChannelTwitchLogo, titleKey: 'settings.channels.twitchTitle', fallback: 'Twitch' },
  { id: 'nostr', logo: ChannelNostrLogo, titleKey: 'settings.channels.nostrTitle', fallback: 'Nostr' },
];

/** Per-platform i18n keys reused from the legacy channel settings page. */
const CREDENTIALS_REQUIRED_KEY: Record<MasterAgentPlatform, string> = {
  telegram: 'settings.assistant.tokenRequired',
  discord: 'settings.discord.tokenRequired',
  slack: 'settings.slack.credentialsRequired',
  matrix: 'settings.matrix.credentialsRequired',
  mattermost: 'settings.mattermost.credentialsRequired',
  twitch: 'settings.twitch.credentialsRequired',
  nostr: 'settings.nostr.credentialsRequired',
  qqbot: 'settings.qqbot.credentialsRequired',
  lark: 'settings.lark.credentialsRequired',
  dingtalk: 'settings.dingtalk.credentialsRequired',
  weixin: 'settings.weixin.loginRequired',
  wecom: 'settings.wecom.configureFirst',
};

const PLUGIN_ENABLED_KEY: Record<MasterAgentPlatform, string> = {
  telegram: 'settings.assistant.pluginEnabled',
  discord: 'settings.discord.pluginEnabled',
  slack: 'settings.slack.pluginEnabled',
  matrix: 'settings.matrix.pluginEnabled',
  mattermost: 'settings.mattermost.pluginEnabled',
  twitch: 'settings.twitch.pluginEnabled',
  nostr: 'settings.nostr.pluginEnabled',
  qqbot: 'settings.qqbot.pluginEnabled',
  lark: 'settings.lark.pluginEnabled',
  dingtalk: 'settings.dingtalk.pluginEnabled',
  weixin: 'settings.weixin.pluginEnabled',
  wecom: 'settings.wecom.pluginEnabled',
};

const PLUGIN_DISABLED_KEY: Record<MasterAgentPlatform, string> = {
  telegram: 'settings.assistant.pluginDisabled',
  discord: 'settings.discord.pluginDisabled',
  slack: 'settings.slack.pluginDisabled',
  matrix: 'settings.matrix.pluginDisabled',
  mattermost: 'settings.mattermost.pluginDisabled',
  twitch: 'settings.twitch.pluginDisabled',
  nostr: 'settings.nostr.pluginDisabled',
  qqbot: 'settings.qqbot.pluginDisabled',
  lark: 'settings.lark.pluginDisabled',
  dingtalk: 'settings.dingtalk.pluginDisabled',
  weixin: 'settings.weixin.pluginDisabled',
  wecom: 'settings.wecom.pluginDisabled',
};

/**
 * Channel config modal body: enable/disable switch + the platform's full
 * config form (credentials, connection test, master-agent switch, pairing
 * approvals, authorized users).
 *
 * 模型选择器在伙伴远程视图里**不渲染**:机器人复用所绑定伙伴的对话模型
 * (后端按绑定伙伴的 profile.model 解析),故这里不向各平台表单传 `modelSelection`,
 * 表单据此隐藏「默认模型」下拉,改在伙伴侧的「对话」页配置。
 *
 * `channelTarget` addresses one channel row (multi-bot model). When
 * `channelTarget.channelId` is missing the form runs in create mode: the
 * first enable creates a new row bound to `channelTarget.companionId`.
 */
const PlatformConfigBody: React.FC<{
  platform: MasterAgentPlatform;
  status: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
  refreshStatuses: () => Promise<void>;
}> = ({ platform, status, channelTarget, onStatusChange, refreshStatuses }) => {
  const { t } = useTranslation();
  const [toggleLoading, setToggleLoading] = useState(false);
  // Telegram lets the user enable with a token typed in the form but not yet saved.
  const telegramTokenRef = useRef('');
  // WeCom's form surfaces callback URLs derived from the WebUI status (best-effort).
  const [webuiStatus, setWebuiStatus] = useState<IWebUIStatus | null>(null);

  useEffect(() => {
    if (platform !== 'wecom') return;
    let cancelled = false;
    webui.getStatus
      .invoke()
      .then((status) => {
        if (!cancelled && status) setWebuiStatus(status);
      })
      .catch(() => {
        // Best-effort only — the form degrades to localhost URLs.
      });
    return () => {
      cancelled = true;
    };
  }, [platform]);

  const handleToggleEnabled = async (enabled: boolean) => {
    setToggleLoading(true);
    try {
      if (enabled) {
        const pendingToken = platform === 'telegram' || platform === 'discord' ? telegramTokenRef.current.trim() : '';
        if (!status?.hasToken && !pendingToken) {
          Message.warning(t(CREDENTIALS_REQUIRED_KEY[platform]));
          return;
        }
        const config = pendingToken ? { credentials: { token: pendingToken } } : {};
        await channel.enablePlugin.invoke(
          channelTarget
            ? { plugin_id: channelTarget.channelId, plugin_type: platform, companion_id: channelTarget.companionId, config }
            : { plugin_id: platform, config }
        );
        Message.success(t(PLUGIN_ENABLED_KEY[platform]));
      } else {
        await channel.disablePlugin.invoke({ plugin_id: channelTarget?.channelId ?? platform });
        Message.success(t(PLUGIN_DISABLED_KEY[platform]));
      }
      await refreshStatuses();
    } catch (error: unknown) {
      Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setToggleLoading(false);
    }
  };

  return (
    <div className='flex flex-col gap-8px py-8px'>
      {/* 渠道启停 / Channel enable-disable */}
      <div className='flex items-center justify-between gap-12px bg-fill-2 rd-10px px-14px py-10px'>
        <div className='min-w-0'>
          <div className='text-14px text-t-primary font-500'>{t('nomi.settings.remoteEnableChannel')}</div>
          <div className='text-12px text-t-tertiary mt-2px'>{t('nomi.settings.remoteEnableChannelHint')}</div>
        </div>
        <Switch
          checked={status?.enabled || false}
          loading={toggleLoading}
          onChange={(checked: boolean) => void handleToggleEnabled(checked)}
        />
      </div>

      {/* 模型跟随伙伴对话模型 / Model follows the partner's chat model */}
      <div className='text-12px text-t-tertiary bg-fill-2 rd-10px px-14px py-10px'>
        机器人使用该伙伴的对话模型,在「对话」页配置
      </div>

      {platform === 'telegram' && (
        <TelegramConfigForm
          pluginStatus={status}
          channelTarget={channelTarget}
          onStatusChange={onStatusChange}
          onTokenChange={(token) => {
            telegramTokenRef.current = token;
          }}
        />
      )}
      {platform === 'discord' && (
        <DiscordConfigForm
          pluginStatus={status}
          channelTarget={channelTarget}
          onStatusChange={onStatusChange}
          onTokenChange={(token) => {
            telegramTokenRef.current = token;
          }}
        />
      )}
      {platform === 'slack' && <SlackConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'matrix' && <MatrixConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'mattermost' && <MattermostConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'twitch' && <TwitchConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'nostr' && <NostrConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'qqbot' && <QQBotConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />}
      {platform === 'lark' && (
        <LarkConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />
      )}
      {platform === 'dingtalk' && (
        <DingTalkConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />
      )}
      {platform === 'weixin' && (
        <WeixinConfigForm pluginStatus={status} channelTarget={channelTarget} onStatusChange={onStatusChange} />
      )}
      {platform === 'wecom' && (
        <WecomConfigForm
          pluginStatus={status}
          channelTarget={channelTarget}
          onStatusChange={onStatusChange}
          webuiStatus={webuiStatus}
        />
      )}
    </div>
  );
};

/**
 * 伙伴设置页「远程连接」节：每伙伴视角的多机器人管理。
 * 每个机器人 = assistant_plugins 一行（行上 companion_id 绑宠，UNIQUE(type,bot_key)
 * 保证同一机器人不绑多宠）。同一平台可以有多行：本宠的行直接启停/配置/解绑/
 * 删除；未绑定的行可以绑到本宠；他宠的行不可抢，但本宠可以为该平台新建自己的
 * 机器人——这是多行模型的核心能力。
 *
 * Per-companion "Remote connect" section over the multi-bot channel model. Each bot
 * is one assistant_plugins row; the card for a platform branches on whether
 * this companion owns a row, an unbound row exists, or only other companions' rows exist.
 * Pending pairing requests still surface as a platform-level badge.
 */
const RemoteConnectSection: React.FC<{ companionId: string; companionName: string }> = ({ companionId, companionName }) => {
  const { t } = useTranslation();
  const { companions } = useCompanions();

  // All channel rows, indexed by row id (NOT platform type — one platform may have many rows).
  const [statuses, setStatuses] = useState<Record<string, IChannelPluginStatus>>({});
  const [pendingCounts, setPendingCounts] = useState<Record<string, number>>({});
  const [busyRowId, setBusyRowId] = useState<string | null>(null);
  // Config modal target: with channelId = edit that row; without = create mode
  // (the form's first save creates a row bound to this companion).
  const [configTarget, setConfigTarget] = useState<{ platform: MasterAgentPlatform; channelId?: string } | null>(null);

  // ── Channel plugin statuses (REST snapshot + WS live updates) ──
  const refreshStatuses = useCallback(async () => {
    try {
      const plugins = await channel.getPluginStatus.invoke();
      if (!plugins) return;
      setStatuses(() => {
        const next: Record<string, IChannelPluginStatus> = {};
        for (const plugin of plugins) next[plugin.id] = plugin;
        return next;
      });
    } catch (error) {
      console.error('[RemoteConnect] Failed to load plugin statuses:', error);
    }
  }, []);

  useEffect(() => {
    void refreshStatuses();
    const unsubscribe = channel.pluginStatusChanged.on(({ status }) => {
      // Merge known rows by id for fast feedback, then reconcile with a REST
      // snapshot: a just-deleted row still emits one trailing enabled=false
      // event, and new rows created elsewhere only exist in the snapshot.
      setStatuses((prev) => (prev[status.id] ? { ...prev, [status.id]: { ...prev[status.id], ...status } } : prev));
      void refreshStatuses();
    });
    return () => unsubscribe();
  }, [refreshStatuses]);

  // ── Pending pairing requests (badge per channel row) ──
  const refreshPendings = useCallback(async () => {
    try {
      const pairings = await channel.getPendingPairings.invoke();
      setPendingCounts(() => {
        const next: Record<string, number> = {};
        for (const pairing of pairings ?? []) {
          if (!pairing.channelId) continue;
          next[pairing.channelId] = (next[pairing.channelId] ?? 0) + 1;
        }
        return next;
      });
    } catch (error) {
      console.error('[RemoteConnect] Failed to load pending pairings:', error);
    }
  }, []);

  useEffect(() => {
    void refreshPendings();
    const unsubs = [
      // New request → badge appears even while the user never opens settings.
      channel.pairingRequested.on(() => void refreshPendings()),
      // Approval resolves a pairing into an authorized user → badge shrinks.
      channel.userAuthorized.on(() => void refreshPendings()),
    ];
    return () => unsubs.forEach((unsubscribe) => unsubscribe());
  }, [refreshPendings]);

  // Adopt the row created from inside a create-mode modal: once a row of that
  // platform bound to this companion shows up, retarget the modal so the enable
  // switch and the form address the new row instead of creating another one.
  useEffect(() => {
    if (!configTarget || configTarget.channelId) return;
    const created = Object.values(statuses).find((s) => s.type === configTarget.platform && s.companionId === companionId);
    if (created) setConfigTarget({ platform: configTarget.platform, channelId: created.id });
  }, [statuses, configTarget, companionId]);

  const companionNameOf = useCallback((id: string | null | undefined) => companions.find((p) => p.id === id)?.name, [companions]);

  // ── Row actions ──
  const handleToggleEnabled = useCallback(
    async (row: IChannelPluginStatus, platform: MasterAgentPlatform, enabled: boolean) => {
      setBusyRowId(row.id);
      try {
        if (enabled) {
          // The outer card has no credential inputs (unlike the config modal's
          // telegram token field) — point the user at the form instead.
          if (!row.hasToken) {
            Message.warning(t(CREDENTIALS_REQUIRED_KEY[platform]));
            return;
          }
          await channel.enablePlugin.invoke({ plugin_id: row.id, config: {} });
          Message.success(t(PLUGIN_ENABLED_KEY[platform]));
        } else {
          await channel.disablePlugin.invoke({ plugin_id: row.id });
          Message.success(t(PLUGIN_DISABLED_KEY[platform]));
        }
        await refreshStatuses();
      } catch (error: unknown) {
        Message.error(error instanceof Error ? error.message : String(error));
      } finally {
        setBusyRowId(null);
      }
    },
    [refreshStatuses, t]
  );

  const applyRowBinding = useCallback(
    async (rowId: string, bind: boolean) => {
      setBusyRowId(rowId);
      try {
        // Backend contract: empty companion_id clears the binding. The call atomically
        // persists the binding AND resets only this channel row's sessions.
        await channel.setMasterAgentCompanion.invoke({ plugin_id: rowId, companion_id: bind ? companionId : null });
        Message.success(
          bind ? t('nomi.settings.remoteBindSuccess', { companionName }) : t('nomi.settings.remoteUnbindSuccess')
        );
        await refreshStatuses();
      } catch (error) {
        console.error(`[RemoteConnect] Failed to update binding for ${rowId}:`, error);
        // Conflict (bot already bound to another companion) carries the other companion's
        // name in the backend message — surface it verbatim.
        if (isBackendHttpError(error) && error.backendMessage) {
          Message.error(error.backendMessage);
        } else {
          Message.error(t('nomi.settings.remoteBindFailed'));
        }
      } finally {
        setBusyRowId(null);
      }
    },
    [companionId, companionName, refreshStatuses, t]
  );

  const confirmBind = useCallback(
    (row: IChannelPluginStatus) => {
      Modal.confirm({
        title: t('nomi.settings.remoteBindRow'),
        content: t('nomi.settings.remoteBindConfirm', { companionName }),
        onOk: () => applyRowBinding(row.id, true),
      });
    },
    [applyRowBinding, companionName, t]
  );

  const confirmUnbind = useCallback(
    (row: IChannelPluginStatus) => {
      Modal.confirm({
        title: t('nomi.settings.remoteUnbindRow'),
        content: t('nomi.settings.remoteUnbindConfirm', { companionName }),
        onOk: () => applyRowBinding(row.id, false),
      });
    },
    [applyRowBinding, companionName, t]
  );

  const confirmDelete = useCallback(
    (row: IChannelPluginStatus) => {
      Modal.confirm({
        title: t('nomi.settings.remoteDeleteBot'),
        content: t('nomi.settings.remoteDeleteConfirm'),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          try {
            await channel.deletePlugin.invoke({ plugin_id: row.id });
            await refreshStatuses();
          } catch (error: unknown) {
            Message.error(error instanceof Error ? error.message : String(error));
          }
        },
      });
    },
    [refreshStatuses, t]
  );

  // ── Row presentation helpers ──
  const statusTag = (row: IChannelPluginStatus | null) => {
    if (!row?.hasToken) {
      return (
        <Tag size='small' color='gray'>
          {t('nomi.settings.remoteStatusNotConfigured')}
        </Tag>
      );
    }
    if (row.enabled && row.connected) {
      return (
        <Tag size='small' color='green'>
          {t('nomi.settings.remoteStatusRunning')}
        </Tag>
      );
    }
    if (row.enabled) {
      return (
        <Tag size='small' bordered={false} className='!bg-primary-1 !text-primary-6'>
          {t('nomi.settings.remoteStatusEnabled')}
        </Tag>
      );
    }
    return (
      <Tag size='small' color='gray'>
        {t('nomi.settings.remoteStatusDisabled')}
      </Tag>
    );
  };

  /** Bot identity line (botUsername preferred over raw botKey), empty when unknown. */
  const botIdentityOf = (row: IChannelPluginStatus | null) => {
    const bot = row?.botUsername || row?.botKey;
    return bot ? t('nomi.settings.remoteBotIdentity', { bot }) : '';
  };

  const allRows = useMemo(() => Object.values(statuses), [statuses]);

  const configChannel = useMemo(
    () => PLATFORMS.find((p) => p.id === configTarget?.platform),
    [configTarget?.platform]
  );

  return (
    <>
      <div className='mt-8px text-13px font-600 text-t-secondary'>{t('nomi.settings.remoteTitle')}</div>
      <div className='text-12px text-t-tertiary -mt-6px'>{t('nomi.settings.remoteHint', { companionName })}</div>

      {PLATFORMS.map(({ id, logo, titleKey, fallback }) => {
        const title = t(titleKey, fallback);
        // Only real DB rows: `GET /plugins` pads every builtin platform with
        // a placeholder entry (id == platform name, hasToken=false) when it
        // has no rows yet. Real rows always carry an encrypted config
        // (hasToken=true) — without this filter an empty platform would be
        // misread as "an unbound bot exists" and offer a binding that 404s.
        const rows = allRows.filter((s) => s.type === id && s.hasToken);
        const myRow = rows.find((r) => r.companionId === companionId);
        const unboundRows = rows.filter((r) => !r.companionId);
        const otherRows = rows.filter((r) => r.companionId && r.companionId !== companionId);
        // The row this card talks about: this companion's bot, else a bindable one.
        const focusRow = myRow ?? unboundRows[0] ?? null;
        // Pending-pairing badge is per channel row (keyed by channelId), so a
        // second bot of the same platform shows its own count, not the platform's.
        const pending = focusRow ? (pendingCounts[focusRow.id] ?? 0) : 0;

        let subtitle = '';
        let actions: React.ReactNode;
        if (myRow) {
          subtitle = botIdentityOf(myRow);
          actions = (
            <>
              <Switch
                checked={myRow.enabled}
                loading={busyRowId === myRow.id}
                onChange={(checked: boolean) => void handleToggleEnabled(myRow, id, checked)}
              />
              <Button size='small' onClick={() => setConfigTarget({ platform: id, channelId: myRow.id })}>
                {t('nomi.settings.remoteConfigure')}
              </Button>
              <Button size='small' onClick={() => confirmUnbind(myRow)}>
                {t('nomi.settings.remoteUnbindRow')}
              </Button>
              <Button size='small' status='danger' onClick={() => confirmDelete(myRow)}>
                {t('nomi.settings.remoteDeleteBot')}
              </Button>
            </>
          );
        } else if (unboundRows.length > 0) {
          const bindable = unboundRows[0];
          subtitle = [t('nomi.settings.remoteUnboundBot'), botIdentityOf(bindable)].filter(Boolean).join(' · ');
          actions = (
            <>
              <Button
                size='small'
                type='primary'
                loading={busyRowId === bindable.id}
                onClick={() => confirmBind(bindable)}
              >
                {t('nomi.settings.remoteBindRow')}
              </Button>
              <Button size='small' onClick={() => setConfigTarget({ platform: id, channelId: bindable.id })}>
                {t('nomi.settings.remoteConfigure')}
              </Button>
            </>
          );
        } else if (otherRows.length > 0) {
          subtitle = t('nomi.settings.remoteOtherBots', {
            num: otherRows.length,
            companions: otherRows.map((r) => companionNameOf(r.companionId) ?? r.companionId).join(', '),
          });
          actions = (
            <Button size='small' onClick={() => setConfigTarget({ platform: id })}>
              {t('nomi.settings.remoteCreateBot')}
            </Button>
          );
        } else {
          actions = (
            <Button size='small' type='primary' onClick={() => setConfigTarget({ platform: id })}>
              {t('nomi.settings.remoteCreateBot')}
            </Button>
          );
        }

        return (
          <div key={id} className='flex items-center gap-16px bg-fill-2 rd-10px px-14px py-12px flex-wrap'>
            <div className='flex items-center gap-10px w-200px shrink-0 min-w-0'>
              <img src={logo} alt={title} className='w-18px h-18px object-contain shrink-0' />
              <div className='min-w-0'>
                <div className='flex items-center gap-6px'>
                  <span className='text-14px text-t-primary font-500 truncate'>{title}</span>
                  {statusTag(focusRow)}
                </div>
                {pending > 0 && (
                  <Tag size='small' color='orangered' className='mt-4px'>
                    {t('nomi.settings.remotePending', { num: pending })}
                  </Tag>
                )}
              </div>
            </div>
            <div className='flex-1 min-w-0 text-12px text-t-tertiary'>{subtitle}</div>
            <div className='flex items-center gap-8px shrink-0'>{actions}</div>
          </div>
        );
      })}

      <NomiModal
        visible={Boolean(configTarget)}
        onCancel={() => {
          setConfigTarget(null);
          // Pairings may have been approved/rejected inside the form.
          void refreshPendings();
          void refreshStatuses();
        }}
        header={{
          title: t('nomi.settings.remoteConfigTitle', {
            channel: configChannel ? t(configChannel.titleKey, configChannel.fallback) : '',
          }),
          showClose: true,
        }}
        footer={null}
        style={{ width: 720 }}
        contentStyle={{ maxHeight: 'calc(80vh - 80px)', padding: '0 2px' }}
      >
        {configTarget && (
          <PlatformConfigBody
            key={configTarget.channelId ?? `${configTarget.platform}:new`}
            platform={configTarget.platform}
            status={configTarget.channelId ? (statuses[configTarget.channelId] ?? null) : null}
            channelTarget={{ channelId: configTarget.channelId, companionId }}
            onStatusChange={(status) => {
              // Forms report the row they saved; merge by row id, then let the
              // snapshot reconcile (create mode discovers the new row there).
              if (status) {
                setStatuses((prev) => ({ ...prev, [status.id]: status }));
              }
              void refreshStatuses();
            }}
            refreshStatuses={refreshStatuses}
          />
        )}
      </NomiModal>
    </>
  );
};

export default RemoteConnectSection;
