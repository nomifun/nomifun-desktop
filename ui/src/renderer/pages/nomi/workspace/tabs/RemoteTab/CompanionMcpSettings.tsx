/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { parseMcpServerId, type CompanionId } from '@/common/types/ids';
import { Button, Message, Select } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';

/** The settings page edits the same MCP selection as the companion composer. */
export default function CompanionMcpSettings({ companionId }: { companionId: CompanionId }) {
  const { t } = useTranslation();
  const [revision, setRevision] = useState(0);
  const [data, setData] = useState<{ companionId: CompanionId; ids: string[]; options: { value: string; label: string }[]; allowed: boolean }>();
  const [error, setError] = useState(false);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
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
        allowed: !conversation || conversation.agent_snapshot?.enabled_capabilities.includes('mcp.connect') === true,
      });
    }).catch(() => { if (!cancelled) setError(true); });
    return () => { cancelled = true; };
  }, [companionId, revision]);
  const save = async (ids: string[]) => {
    if (savingRef.current || ids.length > 16 || data?.companionId !== companionId) return;
    savingRef.current = true;
    setSaving(true);
    try {
      const thread = await ipcBridge.companion.ensureCompanionSession.invoke({ companion_id: companionId });
      await ipcBridge.agentPlatform.sessions.updateMcpSelection.invoke({ agent_session_id: thread.conversation_id, mcp_server_ids: ids.map(parseMcpServerId) });
      setData((previous) => previous?.companionId === companionId ? { ...previous, ids } : previous);
      await refreshConversationCache(thread.conversation_id);
    } catch (cause) {
      Message.error(getConversationCreateErrorMessage(cause, t));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };
  return <NomiSettingSection title={t('agentSettings.resources.kinds.mcpConnection')} description={t(data?.allowed === false ? 'nomi.chat.mcpAgentRequired' : 'nomi.chat.mcpScopeHint')}>
    {error ? <Button onClick={() => setRevision((previous) => previous + 1)}>{t('common.retry')}</Button> : <Select
      aria-label={t('agentSettings.resources.kinds.mcpConnection')} mode='multiple' allowClear
      loading={!data || saving} disabled={!data || data.companionId !== companionId || saving || !data.allowed}
      value={data?.companionId === companionId ? data.ids : []} options={data?.options ?? []}
      onChange={(ids: string[]) => void save(ids)}
    />}
  </NomiSettingSection>;
}
