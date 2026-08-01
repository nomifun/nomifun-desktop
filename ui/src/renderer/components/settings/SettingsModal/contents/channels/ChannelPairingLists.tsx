/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPairingRequest, IChannelUser } from '@/common/types/channel/channel';
import type { ChannelUserId } from '@/common/types/ids';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { Button, Empty, Message, Spin, Tooltip } from '@arco-design/web-react';
import { CheckOne, CloseOne, Copy, Delete, Refresh } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

const SectionHeader: React.FC<{ title: string; action?: React.ReactNode }> = ({ title, action }) => (
  <div className='flex items-center justify-between mb-12px'>
    <h3 className='text-14px font-500 text-t-primary m-0'>{title}</h3>
    {action}
  </div>
);

const formatTime = (timestamp: number) => new Date(timestamp).toLocaleString();

/**
 * Pending pairing-request list shared by the channel config forms.
 * The section visibility gate (`pluginStatus?.enabled && ...`) stays in each form.
 */
export const PendingPairingList: React.FC<{
  pairings: IChannelPairingRequest[];
  loading: boolean;
  onRefresh: () => void;
  onApprove: (code: string) => void;
  onReject: (code: string) => void;
  /** Render the copy-pairing-code button next to the requester name. */
  showCopyButton?: boolean;
}> = ({ pairings, loading, onRefresh, onApprove, onReject, showCopyButton = false }) => {
  const { t } = useTranslation();

  const getRemainingTime = (expiresAt: number) =>
    `${Math.max(0, Math.ceil((expiresAt - Date.now()) / 1000 / 60))} ${t('common.unit.minute_short')}`;

  const copyToClipboard = async (text: string) => {
    try {
      await copyText(text);
      Message.success(t('common.copySuccess', 'Copied'));
    } catch (error) {
      console.error('[ChannelPairing] Copy failed:', error);
    }
  };

  return (
    <div className='bg-fill-1 rd-12px pt-16px pr-16px pb-16px pl-0'>
      <SectionHeader
        title={t('settings.channels.pendingPairings', 'Pending Pairing Requests')}
        action={
          <Button size='mini' type='text' icon={<Refresh size={14} />} loading={loading} onClick={onRefresh}>
            {t('common.refresh', 'Refresh')}
          </Button>
        }
      />
      {loading ? (
        <div className='flex justify-center py-24px'>
          <Spin />
        </div>
      ) : pairings.length === 0 ? (
        <Empty description={t('settings.channels.noPendingPairings', 'No pending pairing requests')} />
      ) : (
        <div className='flex flex-col gap-12px'>
          {pairings.map((pairing) => (
            <div key={pairing.code} className='flex items-center justify-between bg-fill-2 rd-8px p-12px'>
              <div className='flex-1'>
                <div className='flex items-center gap-8px'>
                  <span className='text-14px font-500 text-t-primary'>
                    {pairing.display_name || t('common.unknownUser')}
                  </span>
                  {showCopyButton && (
                    <Tooltip content={t('settings.channels.copyCode', 'Copy pairing code')}>
                      <button
                        className='p-4px bg-transparent border-none text-t-tertiary hover:text-t-primary cursor-pointer'
                        onClick={() => void copyToClipboard(pairing.code)}
                      >
                        <Copy size={14} />
                      </button>
                    </Tooltip>
                  )}
                </div>
                <div className='text-12px text-t-tertiary mt-4px'>
                  {t('settings.channels.pairingCode', 'Code')}:{' '}
                  <code className='bg-fill-3 px-4px rd-2px'>{pairing.code}</code>
                  <span className='mx-8px'>|</span>
                  {t('settings.channels.expiresIn', 'Expires in')}: {getRemainingTime(pairing.expiresAt)}
                </div>
              </div>
              <div className='flex items-center gap-8px'>
                <Button type='primary' size='small' icon={<CheckOne size={14} />} onClick={() => onApprove(pairing.code)}>
                  {t('settings.channels.approve', 'Approve')}
                </Button>
                <Button
                  type='secondary'
                  size='small'
                  status='danger'
                  icon={<CloseOne size={14} />}
                  onClick={() => onReject(pairing.code)}
                >
                  {t('settings.channels.reject', 'Reject')}
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

/**
 * Authorized-user list shared by the channel config forms.
 * The section visibility gate (`pluginStatus?.enabled && users.length > 0`) stays in each form.
 */
export const AuthorizedUserList: React.FC<{
  users: IChannelUser[];
  loading: boolean;
  onRefresh: () => void;
  onRevoke: (channelUserId: ChannelUserId) => void;
  /** Render the platform / authorized-at meta line under the user name. */
  showMeta?: boolean;
}> = ({ users, loading, onRefresh, onRevoke, showMeta = false }) => {
  const { t } = useTranslation();

  return (
    <div className='bg-fill-1 rd-12px pt-16px pr-16px pb-16px pl-0'>
      <SectionHeader
        title={t('settings.channels.authorizedUsers', 'Authorized Users')}
        action={
          <Button size='mini' type='text' icon={<Refresh size={14} />} loading={loading} onClick={onRefresh}>
            {t('common.refresh', 'Refresh')}
          </Button>
        }
      />
      {loading ? (
        <div className='flex justify-center py-24px'>
          <Spin />
        </div>
      ) : users.length === 0 ? (
        <Empty description={t('settings.channels.noAuthorizedUsers', 'No authorized users yet')} />
      ) : (
        <div className='flex flex-col gap-12px'>
          {users.map((user) => (
            <div key={user.channel_user_id} className='flex items-center justify-between bg-fill-2 rd-8px p-12px'>
              <div className='flex-1'>
                <div className='text-14px font-500 text-t-primary'>{user.display_name || t('common.unknownUser')}</div>
                {showMeta && (
                  <div className='text-12px text-t-tertiary mt-4px'>
                    {t('settings.channels.platform', 'Platform')}: {user.platformType}
                    <span className='mx-8px'>|</span>
                    {t('settings.channels.authorizedAt', 'Authorized')}: {formatTime(user.authorizedAt)}
                  </div>
                )}
              </div>
              <Tooltip content={t('settings.channels.revokeAccess', 'Revoke access')}>
                <Button
                  type='text'
                  status='danger'
                  size='small'
                  icon={<Delete size={16} />}
                  onClick={() => onRevoke(user.channel_user_id)}
                />
              </Tooltip>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
