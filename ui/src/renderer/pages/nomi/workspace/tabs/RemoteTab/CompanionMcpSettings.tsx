/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { type CompanionId } from '@/common/types/ids';
import { Button, Select } from '@arco-design/web-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { allowsMultipleMcpServers } from '@/renderer/hooks/agent/agentResourceSelection';

/** The settings page edits the same MCP selection as the companion composer. */
export default function CompanionMcpSettings({ companionId }: { companionId: CompanionId }) {
  const { t } = useTranslation();
  const [revision, setRevision] = useState(0);
  const [data, setData] = useState<{ companionId: CompanionId; ids: string[]; options: { value: string; label: string }[]; allowed: boolean }>();
  const [error, setError] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setData(undefined);
    setError(false);
    void Promise.all([
      ipcBridge.mcpService.listServers.invoke(),
      ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId }).then(async ({ conversation_id }) =>
        conversation_id ? ipcBridge.conversation.get.invoke({ conversation_id }) : undefined),
    ]).then(([servers, conversation]) => {
      if (!cancelled) setData({ companionId,
        ids: conversation?.extra?.mcp_server_ids ?? [],
        options: servers.map((server) => ({ value: server.mcp_server_id, label: server.name, disabled: !server.enabled })),
        allowed: !conversation || allowsMultipleMcpServers(
          conversation.agent_snapshot?.enabled_capabilities ?? []
        ),
      });
    }).catch(() => { if (!cancelled) setError(true); });
    return () => { cancelled = true; };
  }, [companionId, revision]);
  return <NomiSettingSection title={t('agentSettings.resources.kinds.mcpConnection')} description={t(data?.allowed === false ? 'nomi.chat.mcpAgentRequired' : 'nomi.chat.mcpFrozenHint')}>
    {error ? <Button onClick={() => setRevision((previous) => previous + 1)}>{t('common.retry')}</Button> : <Select
      aria-label={t('agentSettings.resources.kinds.mcpConnection')} mode='multiple' allowClear
      loading={!data} disabled
      value={data?.companionId === companionId ? data.ids : []} options={data?.options ?? []}
    />}
  </NomiSettingSection>;
}
