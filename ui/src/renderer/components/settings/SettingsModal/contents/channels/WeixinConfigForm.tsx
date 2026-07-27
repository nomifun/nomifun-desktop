/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPairingRequest, IChannelPluginStatus, IChannelUser } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
import { Button, Empty, Message, Spin, Tooltip } from '@arco-design/web-react';
import { CheckOne, CloseOne, Copy, Delete, Refresh } from '@icon-park/react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { QRCodeSVG } from 'qrcode.react';
import type { ChannelTarget } from './channelTarget';
import {
  applyWeixinAuthorizedUserMutations,
  applyWeixinPairingMutations,
  buildWeixinEnableConfig,
  findWeixinPluginStatusById,
  isWeixinRuntimeConnected,
  type WeixinAuthorizedUserMutation,
  type WeixinPairingMutation,
} from './weixinConfigState';

type LoginState = 'idle' | 'loading_qr' | 'showing_qr' | 'scanned';
const TRANSITIONAL_PLUGIN_STATUSES = new Set(['created', 'initializing', 'ready', 'starting']);

/**
 * Preference row component (local, mirrors other config forms)
 */
const PreferenceRow: React.FC<{
  label: string;
  description?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, description, children }) => (
  <div className='flex items-center justify-between gap-24px py-12px'>
    <div className='flex-1'>
      <span className='text-14px text-t-primary'>{label}</span>
      {description && <div className='text-12px text-t-tertiary mt-2px'>{description}</div>}
    </div>
    <div className='flex items-center'>{children}</div>
  </div>
);

const SectionHeader: React.FC<{ title: string; action?: React.ReactNode }> = ({ title, action }) => (
  <div className='flex items-center justify-between mb-12px'>
    <h3 className='text-14px font-500 text-t-primary m-0'>{title}</h3>
    {action}
  </div>
);

interface WeixinConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  /** 多机器人模式下寻址的渠道行；缺省 = 全局设置页 legacy 单行行为。 */
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

const getRemainingTime = (expiresAt: number, minuteUnit: string) => {
  const remaining = Math.max(0, Math.ceil((expiresAt - Date.now()) / 1000 / 60));
  return `${remaining} ${minuteUnit}`;
};

const formatTime = (timestamp: number) => new Date(timestamp).toLocaleString();

const WeixinConfigForm: React.FC<WeixinConfigFormProps> = ({
  pluginStatus,
  channelTarget,
  onStatusChange,
}) => {
  const { t } = useTranslation();

  const [loginState, setLoginState] = useState<LoginState>('idle');
  // In Electron mode this holds a base64 data URL; in WebUI mode it holds the raw QR ticket string.
  const [qrcodeDataUrl, setQrcodeDataUrl] = useState<string | null>(null);
  // Active `channel.weixin-login` WS subscription disposer for the in-flight
  // login attempt, or null when idle. Replaces the former SSE EventSource ref:
  // EventSource can't carry the desktop's local-trust header, so the QR flow
  // now streams over the (already-trusted) WebSocket instead.
  const unsubscribeRef = useRef<(() => void) | null>(null);

  // Pairing state
  const [pairingLoading, setPairingLoading] = useState(false);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [usersLoading, setUsersLoading] = useState(false);
  const [pendingPairings, setPendingPairings] = useState<IChannelPairingRequest[]>([]);
  const [authorizedUsers, setAuthorizedUsers] = useState<IChannelUser[]>([]);
  const pairingLoadSequenceRef = useRef(0);
  const pairingMutationSequenceRef = useRef(0);
  const pairingMutationsRef = useRef<WeixinPairingMutation[]>([]);
  const usersLoadSequenceRef = useRef(0);
  const authorizedUserMutationSequenceRef = useRef(0);
  const authorizedUserMutationsRef = useRef<WeixinAuthorizedUserMutation[]>([]);
  const runtimeConnected = isWeixinRuntimeConnected(pluginStatus);

  // Drop the WS subscription on unmount to prevent stale callbacks firing.
  useEffect(() => {
    return () => {
      unsubscribeRef.current?.();
      unsubscribeRef.current = null;
    };
  }, []);

  const loadPendingPairings = useCallback(async () => {
    const loadSequence = ++pairingLoadSequenceRef.current;
    const mutationSequenceAtStart = pairingMutationSequenceRef.current;
    setPairingLoading(true);
    setPairingError(null);
    try {
      const pairings = await channel.getPendingPairings.invoke();
      if (loadSequence !== pairingLoadSequenceRef.current) return;

      const snapshot = (pairings ?? []).filter(
        (pairing) =>
          pairing.platformType === 'weixin' &&
          (!channelTarget?.channelPluginId ||
            pairing.channel_plugin_id === channelTarget.channelPluginId)
      );
      const appliedThrough = pairingMutationSequenceRef.current;
      const newerMutations = pairingMutationsRef.current.filter(
        (mutation) =>
          mutation.sequence > mutationSequenceAtStart &&
          mutation.sequence <= appliedThrough
      );

      setPendingPairings(applyWeixinPairingMutations(snapshot, newerMutations));
      pairingMutationsRef.current = pairingMutationsRef.current.filter(
        (mutation) => mutation.sequence > appliedThrough
      );
    } catch (error) {
      if (loadSequence !== pairingLoadSequenceRef.current) return;
      console.error('[WeixinConfig] Failed to load pending pairings:', error);
      setPairingError(
        error instanceof Error && error.message
          ? error.message
          : t('common.unknownError', 'Unknown error')
      );
    } finally {
      if (loadSequence === pairingLoadSequenceRef.current) {
        setPairingLoading(false);
      }
    }
  }, [channelTarget?.channelPluginId, t]);

  const loadAuthorizedUsers = useCallback(async () => {
    const loadSequence = ++usersLoadSequenceRef.current;
    const mutationSequenceAtStart = authorizedUserMutationSequenceRef.current;
    setUsersLoading(true);
    try {
      const users = await channel.getAuthorizedUsers.invoke();
      if (loadSequence !== usersLoadSequenceRef.current) return;

      const snapshot = (users ?? []).filter(
        (user) =>
          user.platformType === 'weixin' &&
          (!channelTarget?.channelPluginId ||
            user.channel_plugin_id === channelTarget.channelPluginId)
      );
      const appliedThrough = authorizedUserMutationSequenceRef.current;
      const newerMutations = authorizedUserMutationsRef.current.filter(
        (mutation) =>
          mutation.sequence > mutationSequenceAtStart &&
          mutation.sequence <= appliedThrough
      );

      setAuthorizedUsers(
        applyWeixinAuthorizedUserMutations(snapshot, newerMutations)
      );
      authorizedUserMutationsRef.current =
        authorizedUserMutationsRef.current.filter(
          (mutation) => mutation.sequence > appliedThrough
        );
    } catch (error) {
      if (loadSequence !== usersLoadSequenceRef.current) return;
      console.error('[WeixinConfig] Failed to load authorized users:', error);
    } finally {
      if (loadSequence === usersLoadSequenceRef.current) {
        setUsersLoading(false);
      }
    }
  }, [channelTarget?.channelPluginId]);

  useEffect(() => {
    void loadPendingPairings();
    void loadAuthorizedUsers();
  }, [loadPendingPairings, loadAuthorizedUsers]);

  // Server channel events are live-only. Re-read the durable projections after
  // reconnect so a pairing/approval emitted while WebSocket was down is not
  // invisible until the user manually refreshes.
  useEffect(() => {
    return channel.reconnected.on(() => {
      void loadPendingPairings();
      void loadAuthorizedUsers();
    });
  }, [loadPendingPairings, loadAuthorizedUsers]);

  // Listen for incoming weixin pairing requests
  useEffect(() => {
    const unsubscribe = channel.pairingRequested.on((request) => {
      if (request.platformType !== 'weixin') return;
      if (channelTarget?.channelPluginId && request.channel_plugin_id !== channelTarget.channelPluginId) return;
      const mutation: WeixinPairingMutation = {
        sequence: ++pairingMutationSequenceRef.current,
        type: 'upsert',
        request,
      };
      pairingMutationsRef.current.push(mutation);
      setPendingPairings((previous) =>
        applyWeixinPairingMutations(previous, [mutation])
      );
    });
    return () => unsubscribe();
  }, [channelTarget?.channelPluginId]);

  // Listen for user authorization
  useEffect(() => {
    const unsubscribe = channel.userAuthorized.on((user) => {
      if (user.platformType !== 'weixin') return;
      if (channelTarget?.channelPluginId && user.channel_plugin_id !== channelTarget.channelPluginId) return;
      const authorizedUserMutation: WeixinAuthorizedUserMutation = {
        sequence: ++authorizedUserMutationSequenceRef.current,
        user,
      };
      authorizedUserMutationsRef.current.push(authorizedUserMutation);
      setAuthorizedUsers((previous) =>
        applyWeixinAuthorizedUserMutations(previous, [
          authorizedUserMutation,
        ])
      );
      const mutation: WeixinPairingMutation = {
        sequence: ++pairingMutationSequenceRef.current,
        type: 'remove-user',
        platformUserId: user.platformUserId,
        channelPluginId: user.channel_plugin_id,
      };
      pairingMutationsRef.current.push(mutation);
      setPendingPairings((previous) =>
        applyWeixinPairingMutations(previous, [mutation])
      );
    });
    return () => unsubscribe();
  }, [channelTarget?.channelPluginId]);

  const handleApprovePairing = async (code: string) => {
    try {
      await channel.approvePairing.invoke({ code });
      Message.success(t('settings.channels.pairingApproved', 'Pairing approved'));
      await loadPendingPairings();
      await loadAuthorizedUsers();
    } catch (error) {
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleRejectPairing = async (code: string) => {
    try {
      await channel.rejectPairing.invoke({ code });
      Message.info(t('settings.channels.pairingRejected', 'Pairing rejected'));
      await loadPendingPairings();
    } catch (error) {
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleRevokeUser = async (channel_user_id: import('@/common/types/ids').ChannelUserId) => {
    try {
      await channel.revokeUser.invoke({ channel_user_id });
      Message.success(t('settings.channels.userRevoked', 'User access revoked'));
      await loadAuthorizedUsers();
    } catch (error) {
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const copyToClipboard = (text: string) => {
    void navigator.clipboard.writeText(text);
    Message.success(t('common.copySuccess', 'Copied'));
  };

  const enableWeixinPlugin = async (
    accountId: string,
    botToken: string,
    baseUrl?: string
  ) => {
    const config = buildWeixinEnableConfig(accountId, botToken, baseUrl);
    const result = await channel.enablePlugin.invoke(
      channelTarget
        ? {
            plugin_id: channelTarget.channelPluginId,
            plugin_type: 'weixin',
            ...(channelTarget.publicAgentId
              ? { public_agent_id: channelTarget.publicAgentId }
              : { companion_id: channelTarget.companionId }),
            config,
          }
        : { plugin_type: 'weixin', config }
    );
    if (!result.success) {
      throw new Error(
        result.error ||
          t('nomi.settings.remoteEnableFailed', {
            defaultValue: 'Failed to enable channel',
          })
      );
    }
    if (!result.plugin_id) {
      throw new Error(t('settings.weixin.enableFailed', 'Failed to enable WeChat plugin'));
    }

    const plugins = await channel.getPluginStatus.invoke();
    const weixinPlugin = findWeixinPluginStatusById(plugins ?? [], result.plugin_id);
    if (!weixinPlugin) {
      throw new Error(t('settings.weixin.enableFailed', 'Failed to enable WeChat plugin'));
    }
    onStatusChange(weixinPlugin);
    setLoginState('idle');
    setQrcodeDataUrl(null);
    Message.success(t('settings.weixin.pluginEnabled', 'WeChat channel enabled'));
  };

  const handleLogin = () => {
    setLoginState('loading_qr');
    setQrcodeDataUrl(null);

    // Tear down any previous attempt's subscription before starting a new one.
    unsubscribeRef.current?.();

    const finishAttempt = () => {
      unsubscribeRef.current?.();
      unsubscribeRef.current = null;
    };

    // Subscribe BEFORE kicking off the flow so we never miss the first `qr`
    // event. The WebSocket is app-wide and already connected; the first event
    // only arrives after the backend's network round-trip to WeChat, so there
    // is ample headroom.
    const unsubscribe = channel.weixinLogin.on((evt) => {
      switch (evt.phase) {
        case 'qr':
          if (evt.qrcodeData) setQrcodeDataUrl(evt.qrcodeData);
          setLoginState('showing_qr');
          break;
        case 'scanned':
          setLoginState('scanned');
          break;
        case 'done':
          finishAttempt();
          enableWeixinPlugin(
            evt.accountId ?? '',
            evt.botToken ?? '',
            evt.baseUrl
          ).catch((err: unknown) => {
            const msg = err instanceof Error ? err.message : String(err);
            Message.error(msg || t('settings.weixin.enableFailed', 'Failed to enable WeChat plugin'));
            setLoginState('idle');
            setQrcodeDataUrl(null);
          });
          break;
        case 'error': {
          finishAttempt();
          const msg = (evt.message ?? '').toLowerCase();
          if (msg.includes('expired') || msg.includes('too many')) {
            Message.warning(t('settings.weixin.loginExpired', 'QR code expired, please try again'));
          } else {
            Message.error(t('settings.weixin.loginError', 'WeChat login failed'));
          }
          setLoginState('idle');
          setQrcodeDataUrl(null);
          break;
        }
      }
    });
    unsubscribeRef.current = unsubscribe;

    channel.startWeixinLogin.invoke().catch((err: unknown) => {
      finishAttempt();
      const msg = err instanceof Error ? err.message : String(err);
      Message.error(msg || t('settings.weixin.loginError', 'WeChat login failed'));
      setLoginState('idle');
      setQrcodeDataUrl(null);
    });
  };

  const handleDisconnect = async () => {
    try {
      const channelPluginId = channelTarget?.channelPluginId ?? pluginStatus?.plugin_id;
      if (!channelPluginId) {
        throw new Error(t('nomi.settings.remoteChannelMissing', 'Channel record is missing'));
      }
      await channel.disablePlugin.invoke({ plugin_id: channelPluginId });
      Message.success(t('settings.weixin.pluginDisabled', 'WeChat channel disabled'));
      onStatusChange(null);
      setLoginState('idle');
      setQrcodeDataUrl(null);
    } catch (error: unknown) {
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const renderLoginArea = () => {
    if (runtimeConnected) {
      return (
        <div className='flex items-center gap-8px'>
          <CheckOne theme='filled' size={16} className='text-green-500' />
          <span className='text-14px text-t-primary'>{t('settings.weixin.connected', 'Connected')}</span>
          {pluginStatus?.botUsername && <span className='text-12px text-t-tertiary'>({pluginStatus.botUsername})</span>}
          <Button
            type='secondary'
            size='small'
            status='danger'
            onClick={() => {
              void handleDisconnect();
            }}
          >
            {t('settings.weixin.disconnect', 'Disconnect')}
          </Button>
        </div>
      );
    }

    if (pluginStatus?.enabled && pluginStatus.hasToken) {
      const isTransitioning = TRANSITIONAL_PLUGIN_STATUSES.has(pluginStatus.status ?? '');
      const statusLabel = isTransitioning
        ? `${t('common.loading', 'Please wait...')} (${pluginStatus.status})`
        : pluginStatus.error ||
          `${t('common.error', 'Error')}${pluginStatus.status ? ` (${pluginStatus.status})` : ''}`;

      return (
        <div className='flex items-center gap-8px'>
          {isTransitioning ? (
            <Spin size={14} />
          ) : (
            <CloseOne theme='filled' size={16} className='text-red-500' />
          )}
          <span
            className={
              isTransitioning ? 'text-14px text-t-secondary' : 'text-14px text-red-500'
            }
          >
            {statusLabel}
          </span>
          <Button
            type='secondary'
            size='small'
            status='danger'
            onClick={() => {
              void handleDisconnect();
            }}
          >
            {t('settings.weixin.disconnect', 'Disconnect')}
          </Button>
        </div>
      );
    }

    if (loginState === 'showing_qr' || loginState === 'scanned') {
      return (
        <div className='flex flex-col items-center gap-8px'>
          {qrcodeDataUrl && <QRCodeSVG value={qrcodeDataUrl} size={160} />}
          {loginState === 'scanned' ? (
            <div className='flex items-center gap-6px text-13px text-t-secondary'>
              <Spin size={14} />
              <span>{t('settings.weixin.scanned', 'Scanned, waiting for confirmation...')}</span>
            </div>
          ) : (
            <span className='text-13px text-t-secondary'>
              {t('settings.weixin.scanPrompt', 'Please scan the QR code with WeChat')}
            </span>
          )}
        </div>
      );
    }

    // idle or loading_qr
    return (
      <Button
        type='primary'
        loading={loginState === 'loading_qr'}
        onClick={() => {
          void handleLogin();
        }}
      >
        {t('settings.weixin.loginButton', 'Scan to Login')}
      </Button>
    );
  };

  return (
    <div className='flex flex-col gap-24px'>
      {/* Login / connection status */}
      <PreferenceRow
        label={t('settings.weixin.accountId', 'Account ID')}
        description={
          !pluginStatus?.hasToken &&
          (loginState === 'idle' || loginState === 'loading_qr')
            ? t('settings.weixin.scanPrompt', 'Please scan the QR code with WeChat')
            : undefined
        }
      >
        {renderLoginArea()}
      </PreferenceRow>

      {/* Next Steps Guide - shown when connected but no authorized users yet */}
      {runtimeConnected && authorizedUsers.length === 0 && (
        <div className='bg-[rgba(var(--primary-rgb),0.08)] rd-12px p-16px border border-[rgba(var(--primary-rgb),0.2)]'>
          <SectionHeader title={t('settings.channels.nextSteps', 'Next Steps')} />
          <div className='text-14px text-t-secondary space-y-8px'>
            <p className='m-0'>
              <strong>1.</strong> {t('settings.weixin.step1', 'Find and send a message to your bot in WeChat')}
            </p>
            <p className='m-0'>
              <strong>2.</strong>{' '}
              {t(
                'settings.weixin.step2',
                'A pairing request will appear below. Click "Approve" to authorize the user.'
              )}
            </p>
            <p className='m-0'>
              <strong>3.</strong>{' '}
              {t(
                'settings.weixin.step3',
                'Once approved, you can start chatting with the AI agent through WeChat!'
              )}
            </p>
          </div>
        </div>
      )}

      {/* Pending Pairing Requests */}
      {(pluginStatus !== null ||
        pairingError !== null ||
        pendingPairings.length > 0) && (
        <div className='bg-fill-1 rd-12px pt-16px pr-16px pb-16px pl-0'>
          <SectionHeader
            title={t('settings.channels.pendingPairings', 'Pending Pairing Requests')}
            action={
              <Button
                size='mini'
                type='text'
                icon={<Refresh size={14} />}
                loading={pairingLoading}
                onClick={loadPendingPairings}
              >
                {t('common.refresh', 'Refresh')}
              </Button>
            }
          />
          {pairingError && (
            <div
              role='alert'
              className='flex items-center justify-between gap-12px bg-red-50 dark:bg-red-900/20 rd-8px p-12px mb-12px'
            >
              <span className='text-13px text-red-600 dark:text-red-400 break-all'>
                {t('common.error', 'Error')}: {pairingError}
              </span>
              <Button
                size='mini'
                status='danger'
                onClick={() => {
                  void loadPendingPairings();
                }}
              >
                {t('common.retry', 'Retry')}
              </Button>
            </div>
          )}
          {pairingLoading && pendingPairings.length === 0 ? (
            <div className='flex justify-center py-24px'>
              <Spin />
            </div>
          ) : pendingPairings.length === 0 ? (
            pairingError ? null : (
              <Empty description={t('settings.channels.noPendingPairings', 'No pending pairing requests')} />
            )
          ) : (
            <div className='flex flex-col gap-12px'>
              {pendingPairings.map((pairing) => (
                <div key={pairing.code} className='flex items-center justify-between bg-fill-2 rd-8px p-12px'>
                  <div className='flex-1'>
                    <div className='flex items-center gap-8px'>
                      <span className='text-14px font-500 text-t-primary'>
                        {pairing.display_name || t('common.unknownUser')}
                      </span>
                      <Tooltip content={t('settings.channels.copyCode', 'Copy pairing code')}>
                        <Button
                          type='text'
                          size='mini'
                          icon={<Copy size={14} />}
                          onClick={() => copyToClipboard(pairing.code)}
                        />
                      </Tooltip>
                    </div>
                    <div className='text-12px text-t-tertiary mt-4px'>
                      {t('settings.channels.pairingCode', 'Code')}:{' '}
                      <code className='bg-fill-3 px-4px rd-2px'>{pairing.code}</code>
                      <span className='mx-8px'>|</span>
                      {t('settings.channels.expiresIn', 'Expires in')}:{' '}
                      {getRemainingTime(pairing.expiresAt, t('common.unit.minute_short'))}
                    </div>
                  </div>
                  <div className='flex items-center gap-8px'>
                    <Button
                      type='primary'
                      size='small'
                      icon={<CheckOne size={14} />}
                      onClick={() => handleApprovePairing(pairing.code)}
                    >
                      {t('settings.channels.approve', 'Approve')}
                    </Button>
                    <Button
                      type='secondary'
                      size='small'
                      status='danger'
                      icon={<CloseOne size={14} />}
                      onClick={() => handleRejectPairing(pairing.code)}
                    >
                      {t('settings.channels.reject', 'Reject')}
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Authorized Users */}
      {authorizedUsers.length > 0 && (
        <div className='bg-fill-1 rd-12px pt-16px pr-16px pb-16px pl-0'>
          <SectionHeader
            title={t('settings.channels.authorizedUsers', 'Authorized Users')}
            action={
              <Button
                size='mini'
                type='text'
                icon={<Refresh size={14} />}
                loading={usersLoading}
                onClick={loadAuthorizedUsers}
              >
                {t('common.refresh', 'Refresh')}
              </Button>
            }
          />
          {usersLoading ? (
            <div className='flex justify-center py-24px'>
              <Spin />
            </div>
          ) : (
            <div className='flex flex-col gap-12px'>
              {authorizedUsers.map((user) => (
                <div key={user.channel_user_id} className='flex items-center justify-between bg-fill-2 rd-8px p-12px'>
                  <div className='flex-1'>
                    <div className='text-14px font-500 text-t-primary'>{user.display_name || t('common.unknownUser')}</div>
                    <div className='text-12px text-t-tertiary mt-4px'>
                      {t('settings.channels.authorizedAt', 'Authorized')}: {formatTime(user.authorizedAt)}
                    </div>
                  </div>
                  <Tooltip content={t('settings.channels.revokeAccess', 'Revoke access')}>
                    <Button
                      type='text'
                      status='danger'
                      size='small'
                      icon={<Delete size={16} />}
                      onClick={() => handleRevokeUser(user.channel_user_id)}
                    />
                  </Tooltip>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default WeixinConfigForm;
