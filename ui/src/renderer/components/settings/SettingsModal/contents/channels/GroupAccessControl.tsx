/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { channel } from '@/common/adapter/ipcBridge';
import {
  buildSetGroupAccessRequest,
  normalizeGroupAccessMode,
  type GroupAccessMode,
} from '@/common/types/channel/channel';
import type { ChannelPluginId } from '@/common/types/ids';
import { Message, Radio } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface GroupAccessControlProps {
  pluginId: ChannelPluginId;
  value: GroupAccessMode;
  onSaved: (mode: GroupAccessMode) => void;
}

const GroupAccessControl: React.FC<GroupAccessControlProps> = ({ pluginId, value, onSaved }) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<GroupAccessMode>(() => normalizeGroupAccessMode(value));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setMode(normalizeGroupAccessMode(value));
  }, [value]);

  const saveMode = async (nextValue: unknown) => {
    const nextMode = normalizeGroupAccessMode(nextValue);
    if (saving || nextMode === mode) return;

    const previousMode = mode;
    setMode(nextMode);
    setSaving(true);
    try {
      await channel.setGroupAccess.invoke(buildSetGroupAccessRequest(pluginId, nextMode));
      onSaved(nextMode);
      Message.success(t('settings.channels.groupAccessSaved'));
    } catch (error: unknown) {
      setMode(previousMode);
      Message.error(
        error instanceof Error && error.message
          ? error.message
          : t('settings.channels.groupAccessSaveFailed')
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <section
      className='flex flex-col gap-12px rd-12px border border-solid border-[var(--color-border-2)] bg-fill-1 p-14px'
      data-testid='channel-group-access-control'
    >
      <div>
        <div className='text-14px font-500 text-t-primary'>
          {t('settings.channels.groupAccessTitle')}
        </div>
        <div className='mt-3px text-12px leading-18px text-t-tertiary'>
          {t('settings.channels.groupAccessDescription')}
        </div>
      </div>

      <Radio.Group
        direction='vertical'
        value={mode}
        disabled={saving}
        onChange={(nextValue) => void saveMode(nextValue)}
      >
        <div className='py-4px'>
          <Radio value='all_members'>
            <span className='text-13px text-t-primary'>
              {t('settings.channels.groupAccessAllMembers')}
            </span>
          </Radio>
          <div className='ml-24px mt-2px text-12px leading-18px text-t-tertiary'>
            {t('settings.channels.groupAccessAllMembersDesc')}
          </div>
        </div>

        <div className='py-4px'>
          <Radio value='allowlist'>
            <span className='text-13px text-t-primary'>
              {t('settings.channels.groupAccessAllowlist')}
            </span>
          </Radio>
          <div className='ml-24px mt-2px text-12px leading-18px text-t-tertiary'>
            {t('settings.channels.groupAccessAllowlistDesc')}
          </div>
        </div>

        <div className='py-4px'>
          <Radio value='disabled'>
            <span className='text-13px text-t-primary'>
              {t('settings.channels.groupAccessDisabled')}
            </span>
          </Radio>
          <div className='ml-24px mt-2px text-12px leading-18px text-t-tertiary'>
            {t('settings.channels.groupAccessDisabledDesc')}
          </div>
        </div>
      </Radio.Group>

      <div className='text-12px leading-18px text-t-secondary bg-fill-2 rd-8px px-10px py-8px'>
        {t('settings.channels.groupAccessMentionHint')}
      </div>

      {mode === 'all_members' && (
        <div className='text-12px leading-18px text-[rgba(var(--orange-7),1)] bg-[rgba(var(--orange-6),0.08)] rd-8px px-10px py-8px'>
          {t('settings.channels.groupAccessTrustedGroupsHint')}
        </div>
      )}
    </section>
  );
};

export default GroupAccessControl;
