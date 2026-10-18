/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Select } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { useMcpServers } from '@/renderer/hooks/mcp/useMcpServers';
import { useMcpDisplayName } from '@/renderer/services/mcp/useMcpDisplayName';
import type { CompanionHandle } from '../../types';

/** One shared default; existing Sessions keep their canonical extension selection. */
export default function CompanionMcpSettings({ companion }: { companion: CompanionHandle }) {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const getDisplayName = useMcpDisplayName();
  const { mcpServers, isMcpServersLoading, mcpServersLoadFailed, retryMcpServers } = useMcpServers();
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const busy = useRef(false);
  const identity = useRef(profile?.companion_id);
  identity.current = profile?.companion_id;
  useEffect(() => { setFailed(false); }, [profile?.companion_id]);
  const servers = mcpServers.filter(server => !server.builtin && server.enabled);
  const selected = profile?.mcp_servers ?? [];
  const save = async (ids: string[]) => {
    if (!profile || busy.current || isMcpServersLoading || mcpServersLoadFailed) return;
    const companionId = profile.companion_id;
    busy.current = true;
    setSaving(true);
    setFailed(false);
    try {
      await patchCompanion({ mcp_servers: ids }, { optimistic: false });
      if (identity.current === companionId) Message.success(t('nomi.cohabit.quickSettings.saved'));
    } catch {
      if (identity.current === companionId) setFailed(true);
    } finally { busy.current = false; setSaving(false); }
  };
  return <NomiSettingSection title={t('agentSettings.resources.kinds.mcpConnection')} description={t('nomi.cohabit.extensionDefaultsHint')}>
    {mcpServersLoadFailed && <div role='alert'>{t('settings.mcpPage.loadFailed')}<Button onClick={retryMcpServers}>{t('common.retry')}</Button></div>}
    <Select aria-label={t('agentSettings.resources.kinds.mcpConnection')} mode='multiple' allowClear
      loading={saving || isMcpServersLoading} disabled={!profile || saving || isMcpServersLoading || mcpServersLoadFailed}
      value={selected}
      options={[
        ...selected.filter(id => !servers.some(server => server.mcp_server_id === id)).map(id => ({ value: id, label: id, disabled: true })),
        ...servers.map(server => ({ value: server.mcp_server_id, label: getDisplayName(server) })),
      ]}
      onChange={(ids: string[]) => void save(ids)} />
    {failed && <p role='alert'>{t('agentSettings.productBinding.saveFailed')}</p>}
  </NomiSettingSection>;
}
