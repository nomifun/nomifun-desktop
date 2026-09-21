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
import { ComposerToolRail } from '@/renderer/components/chat/SessionCapabilityPicker';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import { usePendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
import AgentResourcePicker from '@/renderer/components/agent/AgentResourcePicker';
import {
  agentResourceKindMayRemainUnbound,
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
  type AgentResourceSelectionValue,
} from '@/renderer/hooks/agent/agentResourceSelection';
import { Alert, ConfigProvider } from '@arco-design/web-react';
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
import { useGuidCollaboration } from './hooks/useGuidCollaboration';
import { useGuidInput } from './hooks/useGuidInput';
import { useGuidMention } from './hooks/useGuidMention';
import { useGuidModelSelection } from './hooks/useGuidModelSelection';
import { useGuidPresetCapabilities } from './hooks/useGuidPresetCapabilities';
import { shouldStartGuidCollaboration, useGuidSend } from './hooks/useGuidSend';
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
import { isManagedModelProvider } from '@/common/types/provider/managedModelService';
import GuidModelCompatibilityNotice from './components/GuidModelCompatibilityNotice';
import { modelCapabilityConfigurationRoute } from '@/renderer/pages/modelHub/modelConfigurationRoute';

type GuidNavigationState = {
  resetAgentSelection?: boolean;
  resetSessionOptions?: boolean;
  selectedAgentPresetId?: string;
  selectedAgentTemplateKey?: OfficialPresetKey;
  workspace?: string;
};

const GuidPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const pendingConversation = usePendingConversation();
  const guidContainerRef = useRef<HTMLDivElement>(null);
  const [showFeedbackModal, setShowFeedbackModal] = useState(false);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [resourceSelectionValue, setResourceSelectionValue] = useState<AgentResourceSelectionValue>({});
  const [selectedResourcesAvailable, setSelectedResourcesAvailable] = useState(false);

  useEffect(() => {
    void import('@renderer/pages/conversation');
  }, []);

  const navigationState = location.state as GuidNavigationState | null;
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
  });
  const modelSelection = useGuidModelSelection('nomi');
  const collaboration = useGuidCollaboration(modelSelection.current_model);
  const guidInput = useGuidInput({
    locationState: navigationState,
  });
  const advancedConfig = useGuidSessionOptions();
  const clearSentInput = useCallback(() => {
    guidInput.setInput('');
    guidInput.setFiles([]);
    guidInput.setDir('');
  }, [guidInput.setInput, guidInput.setFiles, guidInput.setDir]);
  const creation = useGuidCreation(agentSelection, guidInput.input, guidInput.files, guidInput.dir, clearSentInput, {
    config: collaboration.config, model: modelSelection.current_model, ready: collaboration.ready,
  });
  useEffect(() => {
    if (creation.draft.pendingPrompt === undefined) return;
    guidInput.setInput(creation.draft.pendingPrompt);
    creation.update(draft => ({ ...draft, pendingPrompt: undefined }));
  }, [creation.draft.pendingPrompt, creation.update, guidInput.setInput]);
  const presetCapabilities = useGuidPresetCapabilities(
    agentSelection.selection.kind === 'preset'
      ? agentSelection.selection.presetId
      : undefined
  );

  const presetResourceResolutionReady = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
    : !presetCapabilities.isLoading && !presetCapabilities.error;
  const presetResourceKinds = agentSelection.selectedTemplate
    ? new Set(agentSelection.selectedTemplate.seed.required_resource_kinds)
    : presetCapabilities.requiredResourceKinds;
  const presetCapabilityIds = agentSelection.selectedTemplate
      ? new Set([
        ...agentSelection.selectedTemplate.seed.enabled_capabilities,
      ].map((selection) => selection.capability.id))
    : presetCapabilities.capabilityIds;
  const presetActionIds = agentSelection.selectedTemplate
    ? new Set(
        agentSelection.selectedTemplate.seed.enabled_capabilities.flatMap(
          (selection) => selection.action_allowlist ?? []
        )
      )
    : presetCapabilities.actionIds;
  const selectedAgentRequiresToolCalls = presetActionIds.size > 0;
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
  const canConfigureCurrentModel = Boolean(
    modelSelection.current_model &&
      !isManagedModelProvider(modelSelection.current_model)
  );
  const collaborationEnabled = presetResourceResolutionReady
    && presetCapabilityIds.has('agent.collaboration');
  // Knowledge is an optional, session-scoped mount. It keeps its compact
  // KnowledgeControl interaction and is applied after the conversation exists;
  // only resources that truly gate launch belong in the large resource picker.
  const knowledgeEnabled =
    presetResourceResolutionReady && presetResourceKinds.has('knowledge_base');
  const resourcePickerKinds = new Set(
    [...presetResourceKinds].filter((kind) => kind !== 'knowledge_base')
  );
  const optionalResourcePickerKinds = requiredAgentResourcePickerKinds(
    [...resourcePickerKinds].filter(agentResourceKindMayRemainUnbound)
  );
  const resourceSelectionResolution = resolveAgentResourceSelections(
    resourcePickerKinds,
    resourceSelectionValue
  );
  const advancedControlsEnabled = presetResourceResolutionReady && presetCapabilityIds.size > 0;
  const idmmControlEnabled = presetResourceResolutionReady;
  const effectiveAutoWork = advancedControlsEnabled ? advancedConfig.autoWork : { enabled: false };
  const isAutoWorkMode = isAutoWorkEntry(effectiveAutoWork);
  const collaborationLaunchConfigured = collaborationEnabled
    && shouldStartGuidCollaboration(collaboration.config);
  const resourceSelectionsReady = presetResourceResolutionReady
    && selectedResourcesAvailable
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
    setResourceSelectionValue({});
    advancedConfig.setKnowledge({
      enabled: false,
      writeback: false,
      writeback_eagerness: 'manual',
      channel_write_enabled: false,
      kb_ids: [],
    });
  }, [advancedConfig.setKnowledge, selectedAgentResourceKey]);
  useEffect(() => {
    if (!presetResourceResolutionReady) return;
    const sourceKey = `${location.key}:${selectedAgentResourceKey}`;
    if (appliedIdmmDefaultRef.current === sourceKey) return;
    advancedConfig.setIdmmDefault(
      agentSelection.selection.kind === 'preset'
        ? presetCapabilities.idmm
        : createDefaultIdmmConfig()
    );
    appliedIdmmDefaultRef.current = sourceKey;
  }, [
    advancedConfig.setIdmmDefault,
    agentSelection.selection.kind,
    location.key,
    presetCapabilities.idmm,
    presetResourceResolutionReady,
    selectedAgentResourceKey,
  ]);

  const mention = useGuidMention({
    presets: agentSelection.presets,
    officialTemplates: agentSelection.officialTemplates,
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
    loading: guidInput.loading,
    selection: agentSelection.selection,
    selectedPreset: agentSelection.selectedPreset,
    selectedTemplate: agentSelection.selectedTemplate,
    current_model: modelSelection.current_model,
    applyAdvancedConfig: (conversationId) =>
      advancedConfig.applyToConversation(conversationId, {
        allowKnowledgeBinding: knowledgeEnabled,
        allowAutomation: advancedControlsEnabled || idmmControlEnabled,
      }),
    autoWork: effectiveAutoWork,
    workspaceEnabled,
    resourceResolutionReady: selectedAgentModelCompatible
      && resourceSelectionsReady
      && (!collaborationEnabled || collaboration.ready),
    collaboration: collaborationEnabled ? collaboration.config : undefined,
    resourceSelections: resourceSelectionResolution.selections,
    setMentionOpen: mention.setMentionOpen,
    setMentionQuery: mention.setMentionQuery,
    setMentionSelectorOpen: mention.setMentionSelectorOpen,
    setMentionActiveIndex: mention.setMentionActiveIndex,
    navigate,
    t,
    beginPending: pendingConversation.begin,
    endPending: pendingConversation.end,
  });

  const handleInputChange = useCallback(
    (value: string) => {
      guidInput.setInput(value);
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
          key={`knowledge-${location.key}`}
          draft={{
            value: advancedConfig.knowledge,
            onChange: advancedConfig.setKnowledge,
          }}
          applyNote={t('guid.advanced.applyNote')}
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
        onSelectModel={(provider, model) => modelSelection.setCurrentModel({ ...provider, use_model: model })}
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
    <CreationComposerContext.Provider value={creation}>
    <ConfigProvider
      getPopupContainer={() => guidContainerRef.current || document.body}
    >
      <div ref={guidContainerRef} className={styles.guidContainer}>
        <div className={styles.guidAdvancedControls}>
          {!creation.draft.mode && advancedControlsNode}
        </div>
        <div className={styles.guidPrimaryStage}>
          <div className={styles.guidLayout}>
            <GuidCompanionShowcase />

            {agentSelection.selection.kind === 'preset' && presetCapabilities.error && (
              <Alert
                type='error'
                showIcon
                title={t('common.error')}
                content={t('agentSettings.errors.presetCapabilitiesLoadFailed')}
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
              {workspaceEnabled && <GuidWorkspaceFootnote workspaceDir={guidInput.dir} onSelectWorkspace={guidInput.setDir} onClearWorkspace={() => guidInput.setDir('')} />}
              <Composer
                sideTools={
                  collaborationEnabled && <ComposerToolRail ariaLabel={t('guid.collaboration.models.label')}>
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
                      disabled={advancedConfig.autoWork.enabled}
                      disabledReason={advancedConfig.autoWork.enabled ? t('guid.collaboration.autoworkExclusive') : undefined}
                    />
                  </ComposerToolRail>
                }
                isFileDragging={guidInput.isFileDragging}
                dragHandlers={guidInput.dragHandlers}
                overlayOpen={mention.mentionOpen}
                header={<ComposerSceneHeader agent={
                  <GuidAgentSelector
                    presets={agentSelection.presets}
                    draftPresets={agentSelection.draftPresets}
                    officialTemplates={agentSelection.officialTemplates}
                    selection={agentSelection.selection}
                    isLoading={agentSelection.isLoading}
                    loadError={agentSelection.loadError}
                    onRetry={agentSelection.refreshPresets}
                    onSelectPreset={(presetId) =>
                      handleSelectAgent({ kind: 'preset', presetId })
                    }
                    onSelectTemplate={(templateKey) =>
                      handleSelectAgent({ kind: 'template', templateKey })
                    }
                  />
                } />}
                beforeInput={<MentionSelectorBadge
                  visible={mention.mentionSelectorVisible}
                  open={mention.mentionSelectorOpen}
                  onOpenChange={mention.setMentionSelectorOpen}
                  agentLabel={mention.selectedAgentLabel}
                  mentionMenu={mentionDropdownNode}
                  onResetQuery={() => mention.setMentionQuery(null)}
                />}
                overlays={mention.mentionOpen && <div className='absolute left-12px right-12px bottom-[calc(100%+8px)] z-70'>{mentionDropdownNode}</div>}
                inputProps={{
                  value: guidInput.input,
                  onChange: handleInputChange,
                  onKeyDown: handleInputKeyDown,
                  onPaste: guidInput.onPaste,
                  onFocus: guidInput.handleTextareaFocus,
                  placeholder: creation.draft.mode ? '描述你想创作的内容，可添加参考素材…' : normalPlaceholder,
                  'data-testid': 'guid-input',
                }}
                attachments={<ComposerAttachments files={guidInput.files} onRemoveFile={guidInput.handleRemoveFile} />}
                tools={<div className='inline-flex items-center gap-6px'>
                  <FileAttachButton openFileSelector={openFileSelector} onLocalFilesAdded={guidInput.handleFilesPasted} showLoadedCapabilities={false} />
                </div>}
                creationTools={<CreationControls prompt={guidInput.input} onPromptChange={guidInput.setInput} files={guidInput.files} />}
                rightTools={<div className='sendbox-responsive-config-group flex flex-1 items-center justify-end gap-2 min-w-0' data-composer-group>{modelSelectorNode}</div>}
                actions={<>
                  <SpeechInputButton disabled={guidInput.loading} locale={i18n.language}
                    onTranscript={transcript => guidInput.setInput(current => appendSpeechTranscript(current, transcript))} />
                  <ComposerSendButton
                    loading={guidInput.loading || creation.loading}
                    disabled={creation.draft.mode ? creation.loading || !creation.ready || !guidInput.input.trim() : isAutoWorkMode ? autoWorkButtonDisabled : send.isButtonDisabled}
                    icon={!creation.draft.mode && isAutoWorkMode ? <Robot theme='filled' size='14' fill='currentColor' strokeWidth={5} /> : undefined}
                    title={!creation.draft.mode && isAutoWorkMode ? t('requirements.autowork.startSession') : undefined}
                    onClick={() => void submitInput()}
                    testId='guid-send-btn'
                  />
                </>}
              />
            </div>

            {!creation.draft.mode && <AgentResourcePicker
              requiredKinds={resourcePickerKinds}
              optionalKinds={optionalResourcePickerKinds}
              capabilityIds={presetCapabilityIds}
              actionIds={presetActionIds}
              value={resourceSelectionValue}
              onChange={setResourceSelectionValue}
              onAvailabilityChange={setSelectedResourcesAvailable}
              disabled={guidInput.loading || !presetResourceResolutionReady}
            />}

            <QuickActionButtons onOpenBugReport={() => setShowFeedbackModal(true)} />
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
