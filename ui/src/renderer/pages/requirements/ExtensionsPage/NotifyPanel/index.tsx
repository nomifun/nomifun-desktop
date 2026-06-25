import React from 'react';
import { useTranslation } from 'react-i18next';
import ChannelList from './ChannelList';
import RoutingRuleList from './RoutingRuleList';

/**
 * NotifyPanel — the unified notification surface of the requirements platform.
 *
 * Brings together two formerly-split concerns in a single scrollable pane:
 *  1. 通知渠道 (CHANNELS): webhook endpoint CRUD (`ChannelList`).
 *  2. 触发规则 (ROUTING RULES): which tag's requirements, on which events,
 *     notify which channel (`RoutingRuleList`).
 */
const NotifyPanel: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className='flex h-full flex-col gap-24px overflow-y-auto p-4px'>
      <section className='flex flex-col gap-12px'>
        <div className='flex flex-col gap-2px'>
          <h2 className='m-0 text-18px font-bold text-t-primary'>{t('requirements.notify.channelsTitle')}</h2>
          <p className='m-0 text-13px text-t-tertiary'>{t('requirements.notify.channelsHint')}</p>
        </div>
        <ChannelList />
      </section>

      <section className='flex flex-col gap-12px'>
        <div className='flex flex-col gap-2px'>
          <h2 className='m-0 text-18px font-bold text-t-primary'>{t('requirements.notify.rulesTitle')}</h2>
          <p className='m-0 text-13px text-t-tertiary'>{t('requirements.notify.rulesHint')}</p>
        </div>
        <RoutingRuleList />
      </section>
    </div>
  );
};

export default NotifyPanel;
