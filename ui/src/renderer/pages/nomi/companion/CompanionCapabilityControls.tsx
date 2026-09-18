/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useRef, useState } from 'react';
import { Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import SessionCapabilityPicker, { useSessionCapabilityCatalog, type SessionCapabilityDraft } from '@/renderer/components/chat/SessionCapabilityPicker';
import { toggleCompanionSkill } from '../workspace/tabs/SkillsTab/companionSkillConfig';
import type { useCompanion } from '../useNomi';
import type { TChatConversation } from '@/common/config/storage';
import { ipcBridge } from '@/common';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { parseMcpServerId } from '@/common/types/ids';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { allowsMultipleMcpServers } from '@/renderer/hooks/agent/agentResourceSelection';

/** Same composer rail and interaction as work sessions; writes companion intent. */
const CompanionCapabilityControls: React.FC<{ companion: ReturnType<typeof useCompanion>; conversation: Extract<TChatConversation, { type: 'nomi' }> }> = ({ companion, conversation }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { catalog, loading, error, retry } = useSessionCapabilityCatalog();
  const { profile, patchCompanion } = companion;
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const mcpEnabled = allowsMultipleMcpServers(
    conversation.agent_snapshot?.enabled_capabilities ?? []
  );
  const draft = useMemo(() => ({
    skillNames: profile ? catalog.skills.filter(({ name }) =>
      (catalog.autoSkillNames.has(name) || profile.skills.enabled.includes(name))
      && !profile.skills.disabled_auto.includes(name)
    ).map(({ name }) => name) : [],
    mcpServerIds: conversation.extra?.mcp_server_ids ?? [],
  }), [catalog, profile, conversation.extra?.mcp_server_ids]);

  const change = async (next: SessionCapabilityDraft) => {
    if (!profile || savingRef.current || loading || error) return;
    let skills = profile.skills;
    for (const { name } of catalog.skills) {
      const checked = next.skillNames.includes(name);
      if (checked !== draft.skillNames.includes(name)) {
        skills = toggleCompanionSkill(skills, catalog.autoSkillNames, name, checked);
      }
    }
    const mcpChanged = [...draft.mcpServerIds].sort().join(',') !== [...next.mcpServerIds].sort().join(',');
    if (mcpChanged && !mcpEnabled) return;
    if (skills === profile.skills && !mcpChanged) return;
    savingRef.current = true;
    setSaving(true);
    try {
      if (mcpChanged) {
        await ipcBridge.agentPlatform.sessions.updateMcpSelection.invoke({
          agent_session_id: conversation.id,
          mcp_server_ids: next.mcpServerIds.map(parseMcpServerId),
        });
        await refreshConversationCache(conversation.id);
      } else {
        await patchCompanion({ skills });
      }
    } catch (error) {
      Message.error(t(mcpChanged && isBackendHttpError(error) && error.status === 409 ? 'nomi.chat.mcpBusy' : 'common.saveFailed'));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  return <SessionCapabilityPicker
    catalog={catalog}
    draft={draft}
    onChange={(next) => void change(next)}
    loading={loading}
    loadFailed={Boolean(error)}
    onRetry={retry}
    disabled={!profile || saving}
    readOnlyKinds={mcpEnabled ? [] : ['mcp']}
    applyMode='next-send'
    applyNote={(kind) => kind === 'skills' ? t('nomi.chat.skillsScopeHint') : t(mcpEnabled ? 'nomi.chat.mcpScopeHint' : 'nomi.chat.mcpAgentRequired')}
    onManage={(kind) => { if (kind === 'mcp') navigate('/mcp'); else if (profile) navigate(`/nomi?companion=${profile.companion_id}&tab=skills`); }}
  />;
};

export default CompanionCapabilityControls;
