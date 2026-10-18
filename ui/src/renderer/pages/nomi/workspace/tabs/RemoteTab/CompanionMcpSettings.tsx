/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { type CompanionId } from '@/common/types/ids';
import { Button, Message, Select } from '@arco-design/web-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { useSessionCapabilitySelection } from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilitySelection';
import { withoutDisabledMcpSelections } from '@/renderer/components/chat/SessionCapabilityPicker/model';
import { useMcpServers } from '@/renderer/hooks/mcp/useMcpServers';
import { useMcpDisplayName } from '@/renderer/services/mcp/useMcpDisplayName';

const SessionMcpSelection = ({ sessionId }: { sessionId: string }) => {
  const { t } = useTranslation();
  const selection = useSessionCapabilitySelection(sessionId);
  const getDisplayName = useMcpDisplayName();
  const { mcpServers: servers, isMcpServersLoading, mcpServersLoadFailed, retryMcpServers } = useMcpServers();
  const visibleDraft = withoutDisabledMcpSelections(selection.draft, servers);
  useEffect(() => {
    if (isMcpServersLoading || mcpServersLoadFailed || selection.loading || selection.saving || selection.error || selection.state?.editable !== true) return;
    if (visibleDraft !== selection.draft) selection.setDraft(visibleDraft);
  }, [isMcpServersLoading, mcpServersLoadFailed, selection, visibleDraft]);
  const error = selection.error?.message ?? (mcpServersLoadFailed ? t('settings.mcpPage.loadFailed') : undefined);
  return <>
    {error && <div role='alert'>{error}<Button onClick={() => { selection.retry(); retryMcpServers(); }}>{t('common.retry')}</Button></div>}
    <Select aria-label={t('agentSettings.resources.kinds.mcpConnection')} mode='multiple' allowClear
      loading={selection.loading || isMcpServersLoading} disabled={selection.state?.editable !== true || selection.saving || isMcpServersLoading || mcpServersLoadFailed}
      value={visibleDraft.mcpServerIds}
      options={servers.filter((server) => !server.builtin && server.enabled).map((server) => ({ value: server.mcp_server_id, label: getDisplayName(server) }))}
      onChange={(ids: string[]) => {
        selection.setDraft({ ...selection.draft, mcpServerIds: ids });
        void selection.applyBeforeSend().catch((cause) => Message.error(cause instanceof Error ? cause.message : String(cause)));
      }} />
  </>;
};

/** Reads and updates the same canonical selection as the companion composer. */
export default function CompanionMcpSettings({ companionId }: { companionId: CompanionId }) {
  const { t } = useTranslation();
  const [revision, setRevision] = useState(0);
  const [sessionId, setSessionId] = useState<string>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setSessionId(undefined);
    setError(undefined);
    void ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId })
      .then(({ conversation_id }) => { if (!cancelled && conversation_id) setSessionId(conversation_id); })
      .catch((cause) => { if (!cancelled) setError(cause instanceof Error ? cause : new Error(String(cause))); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [companionId, revision]);
  return <NomiSettingSection title={t('agentSettings.resources.kinds.mcpConnection')} description={t('conversation.capabilityPicker.nextSendApplyNote')}>
    {error ? <div role='alert'>{error.message}<Button onClick={() => setRevision((value) => value + 1)}>{t('common.retry')}</Button></div>
      : sessionId ? <SessionMcpSelection key={sessionId} sessionId={sessionId} />
      : <Select aria-label={t('agentSettings.resources.kinds.mcpConnection')} loading={loading} disabled mode='multiple' value={[]} />}
  </NomiSettingSection>;
}
