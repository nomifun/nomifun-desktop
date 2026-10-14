/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import Composer, { ComposerSendButton } from '@/renderer/components/chat/Composer';
import ComposerAttachments from '@/renderer/components/chat/ComposerAttachments';
import FileAttachButton from '@/renderer/components/media/FileAttachButton';
import { ComposerSceneHeader } from '@/renderer/creation/ComposerSceneSelector';
import GuidWorkspaceFootnote from './components/GuidWorkspaceFootnote';
import { Robot } from '@icon-park/react';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { isSubmitGesture } from '@/renderer/hooks/chat/useCompositionInput';
import { appendSpeechTranscript } from '@/renderer/hooks/system/useSpeechInput';
import SpeechInputButton from '@/renderer/components/chat/SpeechInputButton';
import SessionCapabilityPicker, { useSessionCapabilityCatalog, type SessionCapabilityDraft } from '@/renderer/components/chat/SessionCapabilityPicker';
import { defaultSessionCapabilityDraft, toSessionCapabilitySelection } from '@/renderer/components/chat/SessionCapabilityPicker/model';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import KnowledgeControl, { defaultKnowledgeBinding } from '@/renderer/pages/conversation/components/KnowledgeControl';
import { useInitialMessage } from '@/renderer/pages/conversation/components/ConversationShell/InitialMessageContext';
import { loadConversationRoute } from '@/renderer/components/layout/routePreload';
import AgentResourcePicker from '@/renderer/components/agent/AgentResourcePicker';
import {
  agentResourceKindMayRemainUnbound,
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
  type AgentResourceSelectionValue,
} from '@/renderer/hooks/agent/agentResourceSelection';
import { Alert, Button, ConfigProvider } from '@arco-design/web-react';
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import GuidAgentSelector from './components/GuidAgentSelector';
import GuidCompanionShowcase from './components/GuidCompanionShowcase';
import ChatModelSelector from '@/renderer/components/chat/ChatModelSelector';
import MentionDropdown, {
  MentionSelectorBadge,
} from './components/MentionDropdown';
import QuickActionButtons from './components/QuickActionButtons';
import {
  autoWorkStartDisabled,
  isAutoWorkEntry,
} from './hooks/autoWorkEntry';
import { useGuidSessionOptions } from './hooks/useGuidSessionOptions';
import { useGuidAgentSelection } from './hooks/useGuidAgentSelection';
import CollaborationComposerControl from '@/renderer/components/collaboration/CollaborationComposerControl';
import { useCollaborationDraft } from '@/renderer/components/collaboration/useCollaborationDraft';
import { shouldStartCollaboration } from '@/renderer/components/collaboration/collaborationSubmission';
import { useGuidInput } from './hooks/useGuidInput';
import { useGuidMention } from './hooks/useGuidMention';
import { useGuidModelSelection } from './hooks/useGuidModelSelection';
import { useGuidPresetCapabilities } from './hooks/useGuidPresetCapabilities';
import { useGuidSend, type GuidSendDeps } from './hooks/useGuidSend';
import { useTypewriterPlaceholder } from './hooks/useTypewriterPlaceholder';
import type { GuidAgentSelection } from './types';
import styles from './index.module.css';
import CreationControls, { CreationModelSelector } from '@/renderer/creation/CreationControls';
import { CreationComposerContext } from '@/renderer/creation/CreationComposerContext';
import { useGuidCreation } from '@/renderer/creation/useGuidCreation';
import type { OfficialPresetKey } from '@/common/types/agentPlatform';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import type { ModelTechnicalCapability } from '@/common/config/storage';
import {
  capabilityOf,
  capabilitySupportsTechnicalCapability,
} from '@/common/utils/providerModels';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import GuidModelCompatibilityNotice from './components/GuidModelCompatibilityNotice';
import { modelCapabilityConfigurationRoute } from '@/renderer/pages/modelHub/modelConfigurationRoute';
import {
  reasoningEffortsForProtocol,
  type SessionReasoningEffort,
} from '@/common/types/reasoningEffort';

type GuidNavigationState = {
  resetAgentSelection?: boolean;
  resetSessionOptions?: boolean;
  selectedAgentPresetId?: string;
  selectedAgentTemplateKey?: OfficialPresetKey;
  workspace?: string;
};

export type GuidProductEntry = {
  templateKey: OfficialPresetKey;
  agentLabel: string;
  workspace?: string;
  placeholder: string;
  createConversation: NonNullable<GuidSendDeps['createConversation']>;
  onConversationCreated: NonNullable<GuidSendDeps['onConversationCreated']>;
  onSubmittingChange?: (submitting: boolean) => void;
  submissionDisabled?: boolean;
};

const GuidPage: React.FC<{ productEntry?: GuidProductEntry }> = ({ productEntry }) => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const initialMessage = useInitialMessage();
  const guidContainerRef = useRef<HTMLDivElement>(null);
  const [showFeedbackModal, setShowFeedbackModal] = useState(false);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [resourceSelectionValue, setResourceSelectionValue] = useState<AgentResourceSelectionValue>({});
  const [resourcePickerReady, setResourcePickerReady] = useState(false);
  const [reasoningEffort, setReasoningEffort] = useState<SessionReasoningEffort>();
  const capabilityCatalog = useSessionCapabilityCatalog();
  const [capabilityDraft, setCapabilityDraft] = useState<SessionCapabilityDraft>({ skillNames: [], mcpServerIds: [] });
  const initializedCapabilities = useRef(false);
  useEffect(() => {
    if (capabilityCatalog.loading || capabilityCatalog.error || initializedCapabilities.current) return;
    initializedCapabilities.current = true;
    setCapabilityDraft(defaultSessionCapabilityDraft(capabilityCatalog.catalog));
  }, [capabilityCatalog.catalog, capabilityCatalog.loading, capabilityCatalog.error]);

  useEffect(() => {
    void loadConversationRoute().catch(() => {});
  }, []);

  // Product surfaces own their fixed Agent and workspace. Ordinary Guid route
  // intents are one-shot navigation state, not the product's persistent setup.
  const navigationState = productEntry ? null : location.state as GuidNavigationState | null;
  const resetAgentRequested =
    navigationState?.resetAgentSelection === true;
  const resetSessionOptionsRequested =
    navigationState?.resetSessionOptions === true;
  const preselectedPresetId = navigationState?.selectedAgentPresetId;
  const preselectedTemplateKey = navigationState?.selectedAgentTemplateKey;

  const agentSelection = useGuidAgentSelection({
    resetAgentSelection: resetAgentRequested,
    selectedAgentPresetId: preselectedPresetId,
    selectedAgentTemplateKey: preselectedTemplateKey,
    locationKey: location.key,
    fixedTemplateKey: productEntry?.templateKey,
  });
  const guidInput = useGuidInput({
    locationState: productEntry ? { workspace: productEntry.workspace } : navigationState,
  });
  const advancedConfig = useGuidSessionOptions();
  const clearSentInput = useCallback(() => {
    guidInput.setInput('');
    guidInput.setFiles([]);
    guidInput.setDir('');
  }, [guidInput.setInput, guidInput.setFiles, guidInput.setDir]);
  const selectedCapabilityPreset = agentSelection.selectedOfficialPreset ?? agentSelection.selectedPreset;
  const selectedCapabilityRevisionDigest = selectedCapabilityPreset?.current_stable_revision?.revision_digest;
  const presetCapabilities = useGuidPresetCapabilities(
    selectedCapabilityPreset?.preset_id,
    selectedCapabilityRevisionDigest,
  );
  // A missing owner configuration can occur before official initialization.
  // Once it exists, its saved document is the sole source for launch preflight.
  const officialSeedFallback = agentSelection.selectedTemplate && !agentSelection.selectedOfficialPreset
    ? agentSelection.selectedTemplate.seed
    : undefined;

  const presetResourceResolutionReady = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
      && (Boolean(officialSeedFallback) || (!presetCapabilities.isLoading && !presetCapabilities.error))
    : !presetCapabilities.isLoading && !presetCapabilities.error;
  const selectedAgentModelKey = selectedCapabilityPreset
    ? `${selectedCapabilityPreset.preset_id}:${selectedCapabilityRevisionDigest ?? 'draft'}`
    : agentSelection.selection.kind === 'template'
      ? `template:${agentSelection.selection.templateKey}`
      : `preset:${agentSelection.selection.presetId}`;
  const modelSelection = useGuidModelSelection('nomi', {
    selectionKey: selectedAgentModelKey,
    ready: presetResourceResolutionReady,
    defaultModel: presetCapabilities.defaultModel,
  });
  const collaboration = useCollaborationDraft(modelSelection.current_model);
  const presetResourceKinds = officialSeedFallback
    ? new Set(officialSeedFallback.required_resource_kinds)
    : presetCapabilities.requiredResourceKinds;
  const presetCapabilityIds = officialSeedFallback
      ? new Set(officialSeedFallback.enabled_capabilities.map((selection) => selection.capability.id))
    : presetCapabilities.capabilityIds;
  const presetActionIds = officialSeedFallback
    ? new Set(
        officialSeedFallback.enabled_capabilities.flatMap(
          (selection) => selection.action_allowlist ?? []
        )
      )
    : presetCapabilities.actionIds;
  const creation = useGuidCreation(agentSelection, guidInput.input, guidInput.files, guidInput.dir, clearSentInput, {
    config: collaboration.config, model: modelSelection.current_model, ready: collaboration.ready,
  }, { requiredResourceKinds: presetResourceKinds, actionIds: presetActionIds, ready: presetResourceResolutionReady }, !productEntry);
  useEffect(() => {
    productEntry?.onSubmittingChange?.(guidInput.loading);
    return () => productEntry?.onSubmittingChange?.(false);
  }, [productEntry?.onSubmittingChange, guidInput.loading]);
  useEffect(() => {
    if (creation.draft.pendingPrompt === undefined) return;
    guidInput.setInput(creation.draft.pendingPrompt);
    creation.update(draft => ({ ...draft, pendingPrompt: undefined }));
  }, [creation.draft.pendingPrompt, creation.update, guidInput.setInput]);
  const selectedAgentRequiresToolCalls = presetActionIds.size > 0 || capabilityDraft.mcpServerIds.length > 0;
  const requiredTechnicalCapabilities: readonly ModelTechnicalCapability[] = selectedAgentRequiresToolCalls
    ? ['function_calling']
    : [];
  const currentModelProvider = modelSelection.modelList.find(
    (provider) => provider.id === modelSelection.current_model?.id
  );
  const currentModelDefinition = currentModelProvider?.models.find(
    (model) => model.model === modelSelection.current_model?.use_model
  );
  const currentModelCapability = currentModelDefinition?.capabilities.find(
    (capability) => capability.task === 'chat'
  );
  const reasoningEffortOptions = capabilitySupportsTechnicalCapability(
    currentModelCapability,
    'reasoning'
  )
    ? reasoningEffortsForProtocol(currentModelCapability?.protocol)
    : [];
  useEffect(() => {
    if (
      currentModelCapability
      && reasoningEffort !== undefined
      && !reasoningEffortOptions.includes(reasoningEffort)
    ) {
      setReasoningEffort(undefined);
    }
  }, [currentModelCapability, reasoningEffort, reasoningEffortOptions]);
  const missingTechnicalCapabilities = requiredTechnicalCapabilities.filter(
    (technical) =>
      !capabilitySupportsTechnicalCapability(currentModelCapability, technical)
  );
  const selectedAgentModelCompatible = missingTechnicalCapabilities.length === 0;
  const selectedAgentModelIncompatible =
    Boolean(modelSelection.current_model) && !selectedAgentModelCompatible;
  const currentModelLabel = modelDisplayLabel(
    modelSelection.current_model?.use_model ?? '',
    currentModelDefinition?.display_name
  );
  const currentProviderLabel =
    currentModelProvider?.name ?? modelSelection.current_model?.name ?? '';
  const compatibleModelCount = modelSelection.modelList.reduce(
    (count, provider) => count + modelSelection.getAvailableModels(provider).filter((model) => {
      const capability = capabilityOf(provider, model, 'chat');
      return requiredTechnicalCapabilities.every((technical) =>
        capabilitySupportsTechnicalCapability(capability, technical)
      );
    }).length,
    0
  );
  const canConfigureCurrentModel = Boolean(modelSelection.current_model);
  const collaborationEnabled = !productEntry && presetResourceResolutionReady
    && presetCapabilityIds.has('agent.collaboration');
  const knowledgeEnabled = presetResourceResolutionReady
    && presetResourceKinds.has('knowledge_base');
  const knowledgeRequiresWrite = [...presetActionIds].some((action) =>
    action === 'knowledge/write' || action === 'knowledge/autogen'
  );
  // Every concrete resource chosen for this launch crosses the same canonical
  // AgentSession admission boundary. Knowledge keeps its compact header control
  // but contributes to this same frozen resource request at Session creation.
  const resourcePickerKinds = new Set(
    [...presetResourceKinds].filter((kind) => kind !== 'knowledge_base' && kind !== 'mcp_server')
  );
  const optionalResourcePickerKinds = requiredAgentResourcePickerKinds(
    [...resourcePickerKinds].filter(agentResourceKindMayRemainUnbound)
  );
  const resourceSelectionResolution = resolveAgentResourceSelections(
    resourcePickerKinds,
    resourceSelectionValue
  );
  const knowledgeResourceSelections = knowledgeEnabled && advancedConfig.knowledge.enabled
    ? advancedConfig.knowledge.kb_ids.map((resourceId) => ({
        resource_kind: 'knowledge_base',
        resource_id: resourceId,
      }))
    : [];
  const sessionResourceSelections = [
    ...resourceSelectionResolution.selections,
    ...knowledgeResourceSelections,
  ];
  const knowledgePolicy = knowledgeResourceSelections.length > 0
    ? {
        writeback: advancedConfig.knowledge.writeback,
        writeback_eagerness: advancedConfig.knowledge.writeback_eagerness,
      }
    : undefined;
  const advancedControlsEnabled = !productEntry && presetResourceResolutionReady && presetCapabilityIds.size > 0;
  const idmmControlEnabled = !productEntry && presetResourceResolutionReady;
  const effectiveAutoWork = advancedControlsEnabled ? advancedConfig.autoWork : { enabled: false };
  const isAutoWorkMode = isAutoWorkEntry(effectiveAutoWork);
  const collaborationLaunchConfigured = collaborationEnabled
    && shouldStartCollaboration(collaboration.config);
  const resourceSelectionsReady = presetResourceResolutionReady
    && resourcePickerReady
    && resourceSelectionResolution.missingKinds.length === 0;
  // A workspace chosen before the Agent target (for example from a project
  // drawer's "new conversation" action) is explicit user intent. Keep that
  // project context visible and bind it on send even when the selected target
  // does not otherwise expose an optional workspace picker.
  const workspaceEnabled = (
    Boolean(guidInput.dir.trim()) ||
    (presetResourceResolutionReady && presetResourceKinds.has('workspace')));
  const hasAgentLaunchTarget = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
    : Boolean(
        agentSelection.selectedPreset?.current_stable_revision &&
          presetResourceResolutionReady
      );
  const hasLaunchTarget = hasAgentLaunchTarget
    && Boolean(modelSelection.current_model)
    && selectedAgentModelCompatible;
  const selectedAgentResourceKey = agentSelection.selection.kind === 'template'
    ? `template:${agentSelection.selection.templateKey}`
    : `preset:${agentSelection.selection.presetId}`;
  const appliedIdmmDefaultRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (productEntry) return;
    setResourceSelectionValue({});
    advancedConfig.setKnowledge(defaultKnowledgeBinding());
  }, [productEntry, advancedConfig.setKnowledge, selectedAgentResourceKey]);
  useEffect(() => {
    if (productEntry) return;
    if (!presetResourceResolutionReady) return;
    const sourceKey = `${location.key}:${selectedAgentResourceKey}:${selectedCapabilityRevisionDigest ?? 'draft'}`;
    if (appliedIdmmDefaultRef.current === sourceKey) return;
    advancedConfig.setIdmmDefault(
      officialSeedFallback ? createDefaultIdmmConfig() : presetCapabilities.idmm
    );
    appliedIdmmDefaultRef.current = sourceKey;
  }, [
    productEntry,
    advancedConfig.setIdmmDefault,
    officialSeedFallback,
    location.key,
    presetCapabilities.idmm,
    presetResourceResolutionReady,
    selectedAgentResourceKey,
    selectedCapabilityRevisionDigest,
  ]);

  const mention = useGuidMention({
    presets: productEntry ? [] : agentSelection.presets,
    officialTemplates: productEntry ? [] : agentSelection.officialTemplates,
    selection: agentSelection.selection,
    setSelection: agentSelection.setSelection,
    selectedPreset: agentSelection.selectedPreset,
    setInput: guidInput.setInput,
  });

  const send = useGuidSend({
    input: guidInput.input,
    setInput: guidInput.setInput,
    files: guidInput.files,
    setFiles: guidInput.setFiles,
    dir: guidInput.dir,
    setDir: guidInput.setDir,
    setLoading: guidInput.setLoading,
    loading: guidInput.loading || Boolean(productEntry?.submissionDisabled),
    selection: agentSelection.selection,
    selectedPreset: agentSelection.selectedPreset,
    selectedTemplate: agentSelection.selectedTemplate,
    current_model: modelSelection.current_model,
    reasoningEffort,
    applyAdvancedConfig: productEntry ? undefined : (conversationId) =>
      advancedConfig.applyToConversation(conversationId, {
        allowAutomation: advancedControlsEnabled || idmmControlEnabled,
      }),
    autoWork: effectiveAutoWork,
    workspaceEnabled,
    resourceResolutionReady: productEntry ? selectedAgentModelCompatible && hasAgentLaunchTarget : selectedAgentModelCompatible
      && resourceSelectionsReady
      && initializedCapabilities.current && !capabilityCatalog.loading && !capabilityCatalog.error
      && (!collaborationEnabled || collaboration.ready),
    collaboration: !productEntry && collaborationEnabled ? collaboration.config : undefined,
    preserveCollaborationDraft: collaboration.copyToConversation,
    resourceSelections: sessionResourceSelections,
    sessionCapabilities: toSessionCapabilitySelection(capabilityDraft),
    knowledgePolicy,
    setMentionOpen: mention.setMentionOpen,
    setMentionQuery: mention.setMentionQuery,
    setMentionSelectorOpen: mention.setMentionSelectorOpen,
    setMentionActiveIndex: mention.setMentionActiveIndex,
    navigate,
    t,
    beginInitialMessage: initialMessage.begin,
    endInitialMessage: initialMessage.end,
    createConversation: productEntry?.createConversation,
    onConversationCreated: productEntry?.onConversationCreated,
  });

  const handleInputChange = useCallback(
    (value: string) => {
      guidInput.setInput(value);
      if (productEntry) return;
      const match = value.match(mention.mentionMatchRegex);
      if (match) {
        mention.setMentionQuery(match[1]);
        mention.setMentionOpen(false);
      } else {
        mention.setMentionQuery(null);
        mention.setMentionOpen(false);
      }
    },
    [
      guidInput.setInput,
      productEntry,
      mention.mentionMatchRegex,
      mention.setMentionOpen,
      mention.setMentionQuery,
    ]
  );

  const [sendKeyPref] = useConfig('chat.sendKey');
  const sendKey = sendKeyPref ?? 'enter';
  const submitInput = creation.draft.mode ? creation.send : send.sendMessageHandler;

  const handleInputKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (
        (mention.mentionOpen || mention.mentionSelectorOpen) &&
        (event.key === 'ArrowDown' || event.key === 'ArrowUp')
      ) {
        event.preventDefault();
        if (mention.filteredMentionOptions.length === 0) return;
        mention.setMentionActiveIndex((previous) => {
          if (event.key === 'ArrowDown') {
            return (
              (previous + 1) % mention.filteredMentionOptions.length
            );
          }
          return (
            (previous - 1 + mention.filteredMentionOptions.length) %
            mention.filteredMentionOptions.length
          );
        });
        return;
      }

      if (
        (mention.mentionOpen || mention.mentionSelectorOpen) &&
        event.key === 'Enter' &&
        !event.shiftKey
      ) {
        event.preventDefault();
        if (mention.filteredMentionOptions.length > 0) {
          const query = mention.mentionQuery?.toLowerCase();
          const exactMatch = query
            ? mention.filteredMentionOptions.find(
                (option) =>
                  option.label.toLowerCase() === query ||
                  option.tokens.has(query)
              )
            : undefined;
          const selected =
            exactMatch ||
            mention.filteredMentionOptions[mention.mentionActiveIndex] ||
            mention.filteredMentionOptions[0];
          if (selected) {
            mention.selectMentionAgent(selected.key);
            return;
          }
        }
        mention.setMentionOpen(false);
        mention.setMentionQuery(null);
        mention.setMentionSelectorOpen(false);
        mention.setMentionActiveIndex(0);
        return;
      }

      if (
        mention.mentionOpen &&
        (event.key === 'Backspace' || event.key === 'Delete') &&
        !mention.mentionQuery
      ) {
        mention.setMentionOpen(false);
        mention.setMentionQuery(null);
        mention.setMentionActiveIndex(0);
        return;
      }

      if (
        !mention.mentionOpen &&
        mention.mentionSelectorVisible &&
        !guidInput.input.trim() &&
        (event.key === 'Backspace' || event.key === 'Delete')
      ) {
        event.preventDefault();
        mention.setMentionSelectorVisible(false);
        mention.setMentionSelectorOpen(false);
        mention.setMentionActiveIndex(0);
        return;
      }

      if (
        (mention.mentionOpen || mention.mentionSelectorOpen) &&
        event.key === 'Escape'
      ) {
        event.preventDefault();
        mention.setMentionOpen(false);
        mention.setMentionQuery(null);
        mention.setMentionSelectorOpen(false);
        mention.setMentionActiveIndex(0);
        return;
      }

      if (isSubmitGesture(event, sendKey)) {
        event.preventDefault();
        if (!guidInput.input.trim() && !isAutoWorkMode) return;
        void submitInput();
      }
    },
    [
      guidInput.input,
      isAutoWorkMode,
      mention,
      sendKey,
      submitInput,
    ]
  );

  const handleSelectAgent = useCallback(
    (selection: GuidAgentSelection) => {
      agentSelection.setSelection(selection);
      advancedConfig.reset();
      collaboration.reset();
      mention.setMentionOpen(false);
      mention.setMentionQuery(null);
      mention.setMentionSelectorOpen(false);
      mention.setMentionActiveIndex(0);
    },
    [
      agentSelection.setSelection,
      advancedConfig.reset,
      collaboration.reset,
      mention.setMentionActiveIndex,
      mention.setMentionOpen,
      mention.setMentionQuery,
      mention.setMentionSelectorOpen,
    ]
  );

  const typewriterPlaceholder = useTypewriterPlaceholder(
    t('conversation.welcome.placeholder')
  );
  const normalPlaceholder = `${mention.selectedAgentLabel}, ${
    typewriterPlaceholder || t('conversation.welcome.placeholder')
  }`;

  useLayoutEffect(() => {
    // Returning from another page resumes the draft. Only an explicit new
    // conversation action requests a reset.
    if (!resetAgentRequested && !resetSessionOptionsRequested) return;
    guidInput.setInput('');
    guidInput.setFiles([]);
    guidInput.setLoading(false);
    if (!navigationState?.workspace) {
      guidInput.setDir('');
    }
    advancedConfig.reset();
    collaboration.reset();
    setReasoningEffort(undefined);
    setResourceSelectionValue({});
    creation.update(draft => ({ ...draft, references: [], pendingPrompt: undefined, pendingFiles: undefined }));
  }, [
    creation.update,
    setResourceSelectionValue,
    advancedConfig.reset,
    collaboration.reset,
    guidInput.setDir,
    guidInput.setFiles,
    guidInput.setInput,
    guidInput.setLoading,
    location.key,
    navigationState?.workspace,
    resetAgentRequested,
    resetSessionOptionsRequested,
  ]);

  useEffect(() => {
    if (!resetAgentRequested && !preselectedPresetId && !preselectedTemplateKey) return;
    if (
      (preselectedPresetId || preselectedTemplateKey) &&
      (agentSelection.isLoading || !agentSelection.isLoaded)
    ) {
      return;
    }
    if ((preselectedPresetId || preselectedTemplateKey) && agentSelection.loadError) return;
    const preselectionResolved =
      (!preselectedPresetId && !preselectedTemplateKey) ||
      (preselectedPresetId
        ? (agentSelection.selection.kind === 'preset' &&
            agentSelection.selection.presetId === preselectedPresetId) ||
          !agentSelection.presets.some(
            (preset) => preset.preset_id === preselectedPresetId
          )
        : (agentSelection.selection.kind === 'template' &&
            agentSelection.selection.templateKey === preselectedTemplateKey) ||
          !agentSelection.officialTemplates.some(
            (template) => template.template_key === preselectedTemplateKey
          ));
    if (!preselectionResolved) return;
    navigate(
      `${location.pathname}${location.search}${location.hash}`,
      { replace: true, state: null }
    );
  }, [
    location.hash,
    location.pathname,
    location.search,
    navigate,
    agentSelection.isLoading,
    agentSelection.isLoaded,
    agentSelection.loadError,
    agentSelection.officialTemplates,
    agentSelection.presets,
    agentSelection.selection,
    preselectedPresetId,
    preselectedTemplateKey,
    resetAgentRequested,
  ]);

  const mentionDropdownNode = (
    <MentionDropdown
      menuRef={mention.mentionMenuRef}
      options={mention.filteredMentionOptions}
      selectedKey={mention.mentionMenuSelectedKey}
      onSelect={mention.selectMentionAgent}
    />
  );

  const advancedControlsNode = knowledgeEnabled || advancedControlsEnabled || idmmControlEnabled ? (
    <>
      {knowledgeEnabled && (
        <KnowledgeControl
          key={`knowledge-${location.key}-${selectedAgentResourceKey}`}
          draft={{
            value: advancedConfig.knowledge,
            onChange: advancedConfig.setKnowledge,
          }}
          writebackAvailable={knowledgeRequiresWrite}
          applyNote={t('knowledge.control.guidApplyNote')}
        />
      )}
      {advancedControlsEnabled && (
        <AutoWorkControl
          key={`autowork-${location.key}`}
          draft={{
            value: advancedConfig.autoWork,
            onChange: advancedConfig.setAutoWork,
          }}
          applyNote={t('guid.advanced.applyNote')}
          disabledReason={collaborationLaunchConfigured ? t('guid.collaboration.autoworkExclusive') : undefined}
        />
      )}
      {idmmControlEnabled && (
        <IdmmControl
          key={`idmm-${location.key}-${selectedAgentResourceKey}`}
          draft={{
            value: advancedConfig.idmm,
            onChange: advancedConfig.setIdmm,
          }}
          applyNote={t('guid.advanced.applyNote')}
        />
      )}
    </>
  ) : null;

  const modelSelectorNode = (
    creation.draft.mode ? <CreationModelSelector files={guidInput.files} /> : (
      <ChatModelSelector
        providers={modelSelection.modelList}
        currentModel={modelSelection.current_model}
        getAvailableModels={modelSelection.getAvailableModels}
        requiredTechnicalCapabilities={requiredTechnicalCapabilities}
        popupVisible={modelPickerOpen}
        onPopupVisibleChange={setModelPickerOpen}
        reasoningEffort={reasoningEffort}
        reasoningEffortOptions={reasoningEffortOptions}
        reasoningEffortDisabled={guidInput.loading}
        onReasoningEffortChange={setReasoningEffort}
        onSelectModel={async (provider, model) => {
          const capability = capabilityOf(provider, model, 'chat');
          const options = capabilitySupportsTechnicalCapability(capability, 'reasoning')
            ? reasoningEffortsForProtocol(capability?.protocol)
            : [];
          if (reasoningEffort !== undefined && !options.includes(reasoningEffort)) {
            setReasoningEffort(undefined);
          }
          await modelSelection.setCurrentModel({ ...provider, use_model: model });
        }}
      />
    )
  );

  const autoWorkButtonDisabled =
    !hasLaunchTarget || !resourceSelectionsReady || !collaboration.ready ||
    autoWorkStartDisabled(guidInput.loading, advancedConfig.autoWork);
  const openFileSelector = () => {
    void ipcBridge.dialog.showOpen.invoke({ properties: ['openFile', 'multiSelections'] })
      .then(paths => { if (paths?.length) guidInput.handleFilesUploaded(paths); })
      .catch(error => console.error('Failed to open file dialog:', error));
  };

  return (
    <CreationComposerContext.Provider value={productEntry ? null : creation}>
    <ConfigProvider
      getPopupContainer={() => guidContainerRef.current || document.body}
    >
      <div ref={guidContainerRef} className={`${styles.guidContainer} ${productEntry ? styles.guidEmbedded : ''}`}>
        <div className={styles.guidAdvancedControls}>
          {!productEntry && !creation.draft.mode && advancedControlsNode}
        </div>
        <div className={styles.guidPrimaryStage}>
          <div className={styles.guidLayout}>
            {!productEntry && <GuidCompanionShowcase />}

            {presetCapabilities.error && (
              <Alert
                type='error'
                showIcon
                title={t('common.error')}
                content={t('agentSettings.errors.presetCapabilitiesLoadFailed')}
                action={<Button size='small' onClick={presetCapabilities.refresh}>
                  {t('agentSettings.actions.retry')}
                </Button>}
                className={styles.guidPresetCapabilityError}
              />
            )}

            {creation.draft.mode && creation.disabledReason && (
              <Alert
                type='warning'
                showIcon
                content={creation.disabledReason}
                className={styles.guidPresetCapabilityError}
              />
            )}

            {selectedAgentModelIncompatible && (
              <GuidModelCompatibilityNotice
                modelLabel={currentModelLabel}
                providerLabel={currentProviderLabel}
                missingCapabilities={missingTechnicalCapabilities}
                compatibleModelCount={compatibleModelCount}
                canConfigureCurrentModel={canConfigureCurrentModel}
                onChooseCompatibleModel={() => setModelPickerOpen(true)}
                onOpenModelConfiguration={() => {
                  const currentModel = modelSelection.current_model;
                  if (!currentModel) return;
                  void navigate(
                    canConfigureCurrentModel
                      ? modelCapabilityConfigurationRoute(currentModel.id, currentModel.use_model)
                      : '/models?section=chat'
                  );
                }}
              />
            )}

            <div className={styles.guidComposerGroup}>
              {!productEntry && workspaceEnabled && <GuidWorkspaceFootnote workspaceDir={guidInput.dir} onSelectWorkspace={guidInput.setDir} onClearWorkspace={() => guidInput.setDir('')} />}
              <Composer
                sideTools={!productEntry &&
                  <SessionCapabilityPicker
                    catalog={capabilityCatalog.catalog}
                    draft={capabilityDraft}
                    onChange={setCapabilityDraft}
                    loading={capabilityCatalog.loading}
                    loadFailed={Boolean(capabilityCatalog.error)}
                    errorMessage={capabilityCatalog.error?.message}
                    onRetry={capabilityCatalog.retry}
                    disabled={guidInput.loading}
                    applyMode='create'
                  >
                    {collaborationEnabled &&
                    <CollaborationComposerControl
                      value={collaboration.activeCollaborators}
                      onChange={collaboration.setCollaborators}
                      mainModel={collaboration.mainModel}
                      selectedTemplate={collaboration.selectedTemplate}
                      workDir={guidInput.dir}
                      onTemplateApply={collaboration.setTemplate}
                      onTemplateClear={() => collaboration.setTemplate(null)}
                      policy={collaboration.policy}
                      onPolicyChange={collaboration.setPolicy}
                      applyNote={t('conversation.capabilityPicker.createApplyNote')}
                      disabled={advancedConfig.autoWork.enabled}
                      disabledReason={advancedConfig.autoWork.enabled ? t('guid.collaboration.autoworkExclusive') : undefined}
                    />}
                  </SessionCapabilityPicker>
                }
                isFileDragging={guidInput.isFileDragging}
                surfaceRef={guidInput.surfaceRef}
                dragHandlers={guidInput.dragHandlers}
                overlayOpen={mention.mentionOpen}
                header={<ComposerSceneHeader sceneSelectionEnabled={!productEntry} agent={
                  productEntry ? <span data-testid='fixed-app-agent'>{productEntry.agentLabel}</span> : <GuidAgentSelector
                    presets={agentSelection.presets}
                    draftPresets={agentSelection.draftPresets}
                    officialTemplates={agentSelection.officialTemplates}
                    selection={agentSelection.selection}
                    isLoading={agentSelection.isLoading}
                    loadError={agentSelection.loadError}
                    onRetry={agentSelection.refreshPresets}
                    resourceConfiguration={resourcePickerKinds.has('browser') && (resourceSelectionValue.browser === undefined || resourceSelectionValue.browser === 'managed-browser') ? (
                      <AgentResourcePicker
                        requiredKinds={['browser']}
                        capabilityIds={presetCapabilityIds}
                        value={resourceSelectionValue}
                        onChange={setResourceSelectionValue}
                        presentation='embedded'
                        disabled={guidInput.loading || !presetResourceResolutionReady}
                      />
                    ) : undefined}
                    onSelectPreset={(presetId) =>
                      handleSelectAgent({ kind: 'preset', presetId })
                    }
                    onSelectTemplate={(templateKey) =>
                      handleSelectAgent({ kind: 'template', templateKey })
                    }
                  />
                } />}
                beforeInput={!productEntry && <MentionSelectorBadge
                  visible={mention.mentionSelectorVisible}
                  open={mention.mentionSelectorOpen}
                  onOpenChange={mention.setMentionSelectorOpen}
                  agentLabel={mention.selectedAgentLabel}
                  mentionMenu={mentionDropdownNode}
                  onResetQuery={() => mention.setMentionQuery(null)}
                />}
                overlays={!productEntry && mention.mentionOpen && <div className='absolute left-12px right-12px bottom-[calc(100%+8px)] z-70'>{mentionDropdownNode}</div>}
                inputProps={{
                  value: guidInput.input,
                  onChange: handleInputChange,
                  onKeyDown: handleInputKeyDown,
                  onPaste: guidInput.onPaste,
                  onFocus: guidInput.handleTextareaFocus,
                  placeholder: productEntry?.placeholder ?? (creation.draft.mode ? '描述你想创作的内容，可添加参考素材…' : normalPlaceholder),
                  'data-testid': 'guid-input',
                }}
                attachments={<ComposerAttachments files={guidInput.files} onRemoveFile={guidInput.handleRemoveFile} />}
                tools={<div className='inline-flex items-center gap-6px'>
                  <FileAttachButton openFileSelector={openFileSelector} onLocalFilesAdded={guidInput.handleFilesPasted} />
                </div>}
                creationTools={!productEntry && <CreationControls prompt={guidInput.input} onPromptChange={guidInput.setInput} files={guidInput.files} />}
                rightTools={<div className='sendbox-responsive-config-group flex flex-1 items-center justify-end gap-2 min-w-0' data-composer-group>{modelSelectorNode}</div>}
                actions={<>
                  <SpeechInputButton disabled={guidInput.loading} locale={i18n.language}
                    onTranscript={transcript => guidInput.setInput(current => appendSpeechTranscript(current, transcript))} />
                  <ComposerSendButton
                    loading={guidInput.loading || creation.loading}
                    disabled={creation.draft.mode ? creation.loading || !creation.ready || !guidInput.input.trim() : isAutoWorkMode ? autoWorkButtonDisabled : send.isButtonDisabled}
                    icon={!creation.draft.mode && isAutoWorkMode ? <Robot theme='filled' size='14' fill='currentColor' strokeWidth={5} /> : undefined}
                    title={creation.draft.mode ? creation.disabledReason : isAutoWorkMode ? t('requirements.autowork.startSession') : undefined}
                    onClick={() => void submitInput()}
                    testId='guid-send-btn'
                  />
                </>}
              />
            </div>

            {!productEntry && !creation.draft.mode && <AgentResourcePicker
              requiredKinds={resourcePickerKinds}
              optionalKinds={optionalResourcePickerKinds}
              capabilityIds={presetCapabilityIds}
              actionIds={presetActionIds}
              value={resourceSelectionValue}
              onChange={setResourceSelectionValue}
              onAdmissionReadinessChange={setResourcePickerReady}
              hideAutomaticBrowser
              disabled={guidInput.loading || !presetResourceResolutionReady}
            />}

            {!productEntry && <QuickActionButtons onOpenBugReport={() => setShowFeedbackModal(true)} />}
          </div>
        </div>

        <FeedbackReportModal
          visible={showFeedbackModal}
          onCancel={() => setShowFeedbackModal(false)}
        />
      </div>
    </ConfigProvider>
    </CreationComposerContext.Provider>
  );
};

export default GuidPage;
