/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Spin, Switch } from '@arco-design/web-react';
import { Connection } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IPublicAgent } from '@/common/adapter/ipcBridge';
import type { ArcoMessageInstance } from '@renderer/utils/ui/useArcoMessage';
import type { MasterAgentPlatform } from '@renderer/components/settings/SettingsModal/contents/channels/channelTarget';
import { SectionCard } from '../components';

interface Props {
  agent: IPublicAgent;
  message: ArcoMessageInstance;
}

/** Bindable IM platforms (mirrors the channel config forms' `MasterAgentPlatform`). */
const PLATFORMS: { id: MasterAgentPlatform; label: string; accent: string }[] = [
  { id: 'telegram', label: 'Telegram', accent: '0,136,204' },
  { id: 'lark', label: '飞书 Lark', accent: '0,127,255' },
  { id: 'wecom', label: '企业微信', accent: '7,193,96' },
  { id: 'weixin', label: '微信', accent: '7,193,96' },
  { id: 'dingtalk', label: '钉钉 DingTalk', accent: '0,122,255' },
  { id: 'qqbot', label: 'QQ 机器人', accent: '18,183,245' },
  { id: 'discord', label: 'Discord', accent: '88,101,242' },
  { id: 'slack', label: 'Slack', accent: '74,21,75' },
  { id: 'matrix', label: 'Matrix', accent: '0,0,0' },
  { id: 'mattermost', label: 'Mattermost', accent: '0,148,204' },
  { id: 'twitch', label: 'Twitch', accent: '145,70,255' },
  { id: 'nostr', label: 'Nostr', accent: '130,80,223' },
];

type BoundState = 'this' | 'other' | 'none';

/**
 * 渠道部署 —— 把这位对外伙伴绑定到 IM 渠道。一个平台的机器人只服务一个对象
 * （对外伙伴或桌面伙伴，互斥）；绑定后陌生人经该渠道自动被安全接待。
 * 机器人凭据仍在「设置 → 渠道」配置；此处只决定该平台由哪位对外伙伴接待。
 */
const ChannelsSection: React.FC<Props> = ({ agent, message }) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  // platform -> the public_agent_id currently bound (null = none/companion).
  const [bound, setBound] = useState<Record<string, string | null>>({});
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    Promise.all(
      PLATFORMS.map((p) =>
        ipcBridge.publicAgent.getChannelBinding
          .invoke({ platform: p.id })
          .then((r) => [p.id, r?.public_agent_id ?? null] as const)
          .catch(() => [p.id, null] as const)
      )
    ).then((pairs) => {
      if (!alive) return;
      setBound(Object.fromEntries(pairs));
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [agent.id]);

  const stateOf = useMemo(
    () =>
      (platform: string): BoundState => {
        const b = bound[platform];
        if (b === agent.id) return 'this';
        if (b) return 'other';
        return 'none';
      },
    [bound, agent.id]
  );

  const toggle = async (platform: MasterAgentPlatform, on: boolean) => {
    setBusy(platform);
    try {
      const r = await ipcBridge.publicAgent.setChannelBinding.invoke({
        platform,
        public_agent_id: on ? agent.id : null,
      });
      setBound((prev) => ({ ...prev, [platform]: r?.public_agent_id ?? null }));
      message.success(
        on
          ? t('publicCompanion.channels.bound', { defaultValue: '已绑定，该渠道现在由这位对外伙伴接待' })
          : t('publicCompanion.channels.unbound', { defaultValue: '已解绑' })
      );
    } catch (e) {
      message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <SectionCard
      icon={<Connection theme='outline' size='16' fill='currentColor' className='block' style={{ lineHeight: 0 }} />}
      title={t('publicCompanion.channels.title', { defaultValue: '渠道部署' })}
      desc={t('publicCompanion.channels.desc', {
        defaultValue:
          '把这位对外伙伴绑定到 IM 渠道，陌生人经该渠道会被自动安全接待（无需逐个通过审批）。一个渠道机器人只服务一个对象。机器人凭据请先在「设置 → 渠道」中配置。',
      })}
    >
      {loading ? (
        <div className='flex justify-center py-32px'>
          <Spin />
        </div>
      ) : (
        <div
          className='grid gap-10px'
          style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(300px, 100%), 1fr))' }}
        >
          {PLATFORMS.map((p) => {
            const st = stateOf(p.id);
            return (
              <div
                key={p.id}
                className='flex items-center gap-12px rd-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-14px py-11px'
              >
                <span
                  className='flex shrink-0 items-center justify-center w-32px h-32px rd-9px text-13px font-700 text-white'
                  style={{ background: `rgb(${p.accent})` }}
                >
                  {p.label.slice(0, 1)}
                </span>
                <div className='min-w-0 flex-1'>
                  <div className='text-13px font-600 text-t-primary truncate'>{p.label}</div>
                  <div className='mt-1px text-11px leading-15px truncate'>
                    {st === 'this' ? (
                      <span className='text-[rgb(var(--success-6))]'>
                        {t('publicCompanion.channels.statusBound', { defaultValue: '由这位对外伙伴接待' })}
                      </span>
                    ) : st === 'other' ? (
                      <span className='text-[rgb(var(--warning-6))]'>
                        {t('publicCompanion.channels.statusOther', { defaultValue: '已绑定其他对外伙伴（开启将改由此接待）' })}
                      </span>
                    ) : (
                      <span className='text-t-tertiary'>
                        {t('publicCompanion.channels.statusNone', { defaultValue: '未绑定' })}
                      </span>
                    )}
                  </div>
                </div>
                <Switch
                  checked={st === 'this'}
                  loading={busy === p.id}
                  disabled={busy === p.id}
                  onChange={(v) => void toggle(p.id, v)}
                />
              </div>
            );
          })}
        </div>
      )}

      <div className='mt-12px text-11px text-t-tertiary leading-16px'>
        {t('publicCompanion.channels.credentialHint', {
          defaultValue: '提示：请先在「设置 → 渠道」中为对应平台配置机器人凭据，绑定后才能真正接待用户。',
        })}
      </div>
    </SectionCard>
  );
};

export default ChannelsSection;
