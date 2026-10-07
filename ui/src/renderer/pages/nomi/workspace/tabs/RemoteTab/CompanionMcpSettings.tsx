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

const SessionMcpSelection = ({ sessionId }: { sessionId: string }) => {
  const { t } = useTranslation();
  const selection = useSessionCapabilitySelection(sessionId);
  const [servers, setServers] = useState<Awaited<ReturnType<typeof ipcBridge.mcpService.listServers.invoke>>>([]);
  const [loadError, setLoadError] = useState<Error>();
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let cancelled = false;
    setLoadError(undefined);
    void ipcBridge.mcpService.listServers.invoke().then((next) => { if (!cancelled) setServers(next); })
      .catch((cause) => { if (!cancelled) setLoadError(cause instanceof Error ? cause : new Error(String(cause))); });
    return () => { cancelled = true; };
  }, [reload]);
  const error = selection.error ?? loadError;
  return <>
    {error && <div role='alert'>{error.message}<Button onClick={() => { selection.retry(); setReload((value) => value + 1); }}>{t('common.retry')}</Button></div>}
    <Select aria-label={t('agentSettings.resources.kinds.mcpConnection')} mode='multiple' allowClear
      loading={selection.loading} disabled={selection.state?.editable !== true || selection.saving || Boolean(loadError)}
      value={selection.draft.mcpServerIds}
      options={servers.filter((server) => !server.builtin).map((server) => ({ value: server.mcp_server_id, label: server.name, disabled: !server.enabled }))}
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
