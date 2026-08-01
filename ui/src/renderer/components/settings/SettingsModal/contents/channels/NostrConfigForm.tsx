/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
import { Button, Input, Message } from '@arco-design/web-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildEnablePluginRequest, findEnabledChannelStatus } from '@/renderer/components/channels/channelStatusSelection';
import { AuthorizedUserList, PendingPairingList } from './ChannelPairingLists';
import type { ChannelTarget } from './channelTarget';
import { useChannelPairing } from './useChannelPairing';

interface NostrConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

const DEFAULT_RELAYS = 'wss://relay.damus.io,wss://nos.lol';

/** Nostr config: private key (nsec/hex) + relay list. NIP-04 encrypted DMs. */
const NostrConfigForm: React.FC<NostrConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange }) => {
  const { t } = useTranslation();
  const [privateKey, setPrivateKey] = useState('');
  const [relays, setRelays] = useState(DEFAULT_RELAYS);
  const [testLoading, setTestLoading] = useState(false);

  const {
    pendingPairings,
    authorizedUsers,
    pairingLoading,
    usersLoading,
    loadPendingPairings,
    loadAuthorizedUsers,
    approvePairing,
    rejectPairing,
    revokeUser,
  } = useChannelPairing('nostr', channelTarget);

  const handleAutoEnable = async () => {
    const config = { credentials: { nostr_private_key: privateKey.trim(), nostr_relays: relays.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('nostr', channelTarget, config));
    if (!result.success) {
      throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
    }
    Message.success(t('settings.nostr.pluginEnabled', 'Nostr bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      const row = findEnabledChannelStatus(plugins, {
        platform: 'nostr',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      onStatusChange(row || null);
    }
  };

  const handleTestConnection = async () => {
    if (!privateKey.trim() || !relays.trim()) {
      Message.warning(t('settings.nostr.credentialsRequired', 'Please enter a private key and at least one relay'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'nostr', token: privateKey.trim(), extra_config: { nostr_relays: relays.trim() } });
      if (result.success) {
        Message.success(t('settings.nostr.connectionSuccess', { defaultValue: 'Key OK: {{username}}', username: result.bot_username || 'npub' }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.nostr.connectionFailed', 'Invalid key'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.nostr.connectionFailed', 'Invalid key'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.nostr.privateKey', 'Private Key (nsec / hex)')}</span>
        <Input.Password value={privateKey} onChange={setPrivateKey} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'nsec1... or 64-char hex'} visibilityToggle disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.nostr.relays', 'Relays (comma-separated)')}</span>
        <Input value={relays} onChange={setRelays} placeholder={DEFAULT_RELAYS} disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.nostr.tokensDesc', 'Provide a Nostr private key (nsec or hex) — it is the bot identity — and a comma-separated list of relay URLs. Encrypted (NIP-04) direct messages only.')}</div>
        <div>
          <Button type='outline' loading={testLoading} onClick={handleTestConnection} disabled={credentialsLocked}>
            {t('settings.channels.testConnection', 'Test')}
          </Button>
        </div>
      </div>

      {pluginStatus?.enabled && authorizedUsers.length === 0 && (
        <PendingPairingList
          pairings={pendingPairings}
          loading={pairingLoading}
          onRefresh={loadPendingPairings}
          onApprove={approvePairing}
          onReject={rejectPairing}
        />
      )}

      {pluginStatus?.enabled && authorizedUsers.length > 0 && (
        <AuthorizedUserList
          users={authorizedUsers}
          loading={usersLoading}
          onRefresh={loadAuthorizedUsers}
          onRevoke={revokeUser}
        />
      )}
    </div>
  );
};

export default NostrConfigForm;
