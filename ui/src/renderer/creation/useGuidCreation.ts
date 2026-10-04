import type { TProviderWithModel } from '@/common/config/storage';
import type { GuidCollaborationConfig } from '@/renderer/pages/guid/hooks/useGuidCollaboration';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { parseConversationId, type ConversationId } from '@/common/types/ids';
import type { useGuidAgentSelection } from '@/renderer/pages/guid/hooks/useGuidAgentSelection';
import { prepareOfficialAgent } from '@/renderer/pages/guid/hooks/officialAgentLaunch';
import { resolveAgentResourceSelections } from '@/renderer/hooks/agent/agentResourceSelection';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { emitter } from '@/renderer/utils/emitter';
import { useCreationDraft, creationDraftStorageKey } from './useCreationDraft';
import { useGenerationModel } from './useGenerationModel';
import { buildCreationRequest, creationAttempt, acknowledgeCreationAttempt } from './submission';
import { submitCreation } from './client';
import type { CreationMode } from './types';
import { agentBrowserStorageGenerationKey } from '@/common/utils/browserStorageKey';

export function useGuidCreation(agent: ReturnType<typeof useGuidAgentSelection>, input: string, files: string[], workspace: string, onAccepted?: () => void, sessionCollaboration?: { config?: GuidCollaborationConfig; model?: TProviderWithModel; ready: boolean }) {
  // Generation admission is fully frozen by the Creative Studio Agent and its
  // asset-library resource. Workspace/collaboration drafts belong to chat
  // Sessions and must never mutate this immutable Session after creation.
  void workspace;
  void sessionCollaboration;
  const creation = useCreationDraft('guid');
  const model = useGenerationModel(creation, files);
  const location = useLocation();
  const navigate = useNavigate();
  const [loading, setLoading] = useState(false);
  const sending = useRef(false);
  const pendingSession = useRef<ConversationId | null>(null);
  const isCreative = agent.selection.kind === 'template' && agent.selection.templateKey === 'creative-studio.default';
  const selectMode = useCallback((mode: CreationMode) => {
    agent.setSelection({ kind: 'template', templateKey: 'creative-studio.default' });
    creation.setMode(mode);
  }, [agent.setSelection, creation.setMode]);
  const exit = useCallback(() => {
    creation.setMode(null);
    agent.setSelection({ kind: 'template', templateKey: 'assistant.general' });
  }, [agent.setSelection, creation.setMode]);
  const selectionKey = JSON.stringify(agent.selection);
  useEffect(() => {
    if (isCreative && !creation.draft.mode) creation.setMode(creation.draft.lastMode);
    else if (!isCreative) {
      creation.setMode(null);
    }
  }, [selectionKey]);
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const mode = params.get('creation');
    if (mode === 'image' || mode === 'video' || mode === 'music') {
      selectMode(mode);
      // Consume the entry action so Back/remount cannot overwrite a later choice.
      params.delete('creation');
      void navigate({ pathname: location.pathname, search: params.toString(), hash: location.hash }, { replace: true, state: location.state });
    }
  }, [location.key, location.search]);
  const send = useCallback(async () => {
    if (sending.current) return;
    if (!model.ready) { Message.error('请选择可用的生成模型'); return; }
    sending.current = true;
    setLoading(true);
    try {
      const template = agent.officialTemplates.find(value => value.template_key === 'creative-studio.default');
      if (!template) throw new Error('创意工坊 Agent 尚未加载，请重试');
      const resources = resolveAgentResourceSelections(template.seed.required_resource_kinds, {});
      if (resources.missingKinds.length) throw new Error('创意工坊所需资源尚未就绪，请刷新后重试');
      const preset = await prepareOfficialAgent(template, '创意工坊');
      const request = buildCreationRequest(creation.draft, input, preset.preset_id, files, model.selected);
      const pendingSessionKey = agentBrowserStorageGenerationKey('creation-guid-pending-session');
      if (!pendingSession.current) {
        const stored = sessionStorage.getItem(pendingSessionKey);
        if (stored) pendingSession.current = parseConversationId(stored);
      }
      if (!pendingSession.current) {
        const session = await ipcBridge.agentPlatform.sessions.create.invoke({ preset_id: preset.preset_id, title: input.trim().slice(0, 80), resource_selections: resources.selections });
        pendingSession.current = parseConversationId(session.agent_session_id);
        sessionStorage.setItem(pendingSessionKey, pendingSession.current);
      }
      const id = pendingSession.current;
      const key = creationAttempt(id, request);
      await submitCreation(id, request, key);
      acknowledgeCreationAttempt(id, key);
      try {
        sessionStorage.setItem(creationDraftStorageKey(id), JSON.stringify({ ...creation.draft, references: creation.draft.references.filter(ref => !request.inputs.some(input => input.asset_id === ref.asset_id)), pendingFiles: files.filter(file => !request.files?.includes(file)), selectedAgent: { kind: 'template', templateKey: 'creative-studio.default' }, presetId: preset.preset_id, agentLabel: '创意工坊' }));
        sessionStorage.removeItem(pendingSessionKey);
      } catch { /* An accepted task still opens its canonical conversation. */ }
      pendingSession.current = null;
      creation.update(draft => ({ ...draft, references: [], pendingPrompt: undefined, pendingFiles: undefined }));
      onAccepted?.();
      const conversation = await ipcBridge.conversation.get.invoke({ conversation_id: id }).catch(() => null);
      if (conversation) seedConversationCache(conversation);
      emitter.emit('chat.history.refresh');
      await navigate(`/conversation/${id}`);
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { sending.current = false; setLoading(false); }
  }, [agent.officialTemplates, creation.draft, creation.update, files, input, model.ready, model.selected, navigate, onAccepted]);
  return { ...creation, selectMode, exit, send, loading, ready: model.ready };
}
