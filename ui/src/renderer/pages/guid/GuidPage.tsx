/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import Composer, { ComposerSendButton } from '@/renderer/components/chat/Composer';
import ComposerAttachments from '@/renderer/components/chat/ComposerAttachments';
import FileAttachButton from '@/renderer/components/media/FileAttachButton';
import { ComposerSceneHeader, SceneDiscoveryHint } from '@/renderer/creation/ComposerSceneSelector';
import GuidWorkspaceFootnote from './components/GuidWorkspaceFootnote';
import { Robot } from '@icon-park/react';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { isSubmitGesture } from '@/renderer/hooks/chat/useCompositionInput';
import { appendSpeechTranscript } from '@/renderer/hooks/system/useSpeechInput';
import SpeechInputButton from '@/renderer/components/chat/SpeechInputButton';
import SessionCapabilityPicker, {
  buildSessionCapabilitySelection,
  defaultSessionCapabilityDraft,
  useSessionCapabilityCatalog,
  type SessionCapabilityDraft,
} from '@/renderer/components/chat/SessionCapabilityPicker';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import { usePendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
import AgentResourcePicker from '@/renderer/components/agent/AgentResourcePicker';
import {
  resolveAgentResourceSelections,
  selectedMcpResourceIds,
  hasFrozenMcpTools,
  type AgentResourceSelectionValue,
} from '@/renderer/hooks/agent/agentResourceSelection';
import { Alert, Button, ConfigProvider } from '@arco-design/web-react';
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
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
import { useGuidAdvancedConfig } from './hooks/useGuidAdvancedConfig';
import { useGuidAgentSelection } from './hooks/useGuidAgentSelection';
import CollaborationComposerControl from '@/renderer/components/collaboration/CollaborationComposerControl';
import { useGuidCollaboration } from './hooks/useGuidCollaboration';
import { useGuidInput } from './hooks/useGuidInput';
import { useGuidMention } from './hooks/useGuidMention';
import { useGuidModelSelection } from './hooks/useGuidModelSelection';
import { useGuidPresetCapabilities } from './hooks/useGuidPresetCapabilities';
import { useGuidSend } from './hooks/useGuidSend';
import { useTypewriterPlaceholder } from './hooks/useTypewriterPlaceholder';
import type { GuidAgentSelection } from './types';
import styles from './index.module.css';
import CreationControls, { CreationModelSelector } from '@/renderer/creation/CreationControls';
import { CreationComposerContext } from '@/renderer/creation/CreationComposerContext';
import { useGuidCreation } from '@/renderer/creation/useGuidCreation';
import { useCompanion } from '@/renderer/pages/nomi/useNomi';
import { parseCompanionId } from '@/common/types/ids';

type GuidNavigationState = {
  resetAgentSelection?: boolean;
  selectedAgentPresetId?: string;
  workspace?: string;
};

const GuidPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const pendingConversation = usePendingConversation();
  const guidContainerRef = useRef<HTMLDivElement>(null);
  const [showFeedbackModal, setShowFeedbackModal] = useState(false);
  const [resourceSelectionValue, setResourceSelectionValue] = useState<AgentResourceSelectionValue>({});
  const [selectedResourcesAvailable, setSelectedResourcesAvailable] = useState(false);
  const [capabilityDraft, setCapabilityDraft] = useState<SessionCapabilityDraft>({
    skillNames: [],
    mcpServerIds: [],
  });
  const capabilityCatalog = useSessionCapabilityCatalog();

  useEffect(() => {
    void import('@renderer/pages/conversation');
  }, []);

  const navigationState = location.state as GuidNavigationState | null;
  const resetAgentRequested =
    navigationState?.resetAgentSelection === true;
  const preselectedPresetId = navigationState?.selectedAgentPresetId;

  const agentSelection = useGuidAgentSelection({
    resetAgentSelection: resetAgentRequested,
    selectedAgentPresetId: preselectedPresetId,
    locationKey: location.key,
  });
  const modelSelection = useGuidModelSelection('nomi');
  const collaboration = useGuidCollaboration(modelSelection.current_model);
  const guidInput = useGuidInput({
    locationState: navigationState,
  });
  const advancedConfig = useGuidAdvancedConfig();
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
  // Knowledge is an optional, session-scoped mount. It keeps its compact
  // KnowledgeControl interaction and is applied after the conversation exists;
  // only resources that truly gate launch belong in the large resource picker.
  const isCompanionAgent = agentSelection.selection.kind === 'template' && agentSelection.selection.templateKey === 'companion.default';
  const selectedCompanion = useCompanion(isCompanionAgent && resourceSelectionValue.companion ? parseCompanionId(resourceSelectionValue.companion) : null);
  const knowledgeEnabled =
    !isCompanionAgent && presetResourceResolutionReady && presetResourceKinds.has('knowledge_base');
  const resourcePickerKinds = new Set(
    [...presetResourceKinds].filter((kind) => kind !== 'knowledge_base')
  );
  const resourceSelectionResolution = resolveAgentResourceSelections(
    resourcePickerKinds,
    resourceSelectionValue
  );
  const advancedControlsEnabled = !isCompanionAgent && presetResourceResolutionReady && presetCapabilityIds.size > 0;
  const effectiveAutoWork = advancedControlsEnabled ? advancedConfig.autoWork : { enabled: false };
  const isAutoWorkMode = isAutoWorkEntry(effectiveAutoWork);
  const resourceSelectionsReady = presetResourceResolutionReady
    && selectedResourcesAvailable
    && resourceSelectionResolution.missingKinds.length === 0;
  // A workspace chosen before the Agent target (for example from a project
  // drawer's "new conversation" action) is explicit user intent. Keep that
  // project context visible and bind it on send even when the selected target
  // does not otherwise expose an optional workspace picker.
  const workspaceEnabled = !isCompanionAgent && (
    Boolean(guidInput.dir.trim()) ||
    (presetResourceResolutionReady && presetResourceKinds.has('workspace')));
  const hasAgentLaunchTarget = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
    : Boolean(
        agentSelection.selectedPreset?.current_stable_revision &&
          presetResourceResolutionReady
      );
  const hasLaunchTarget = hasAgentLaunchTarget && Boolean(modelSelection.current_model);
  const selectedAgentResourceKey = agentSelection.selection.kind === 'template'
    ? `template:${agentSelection.selection.templateKey}`
    : `preset:${agentSelection.selection.presetId}`;
  const presetSkillNames = agentSelection.selectedTemplate
    ? new Set(agentSelection.selectedTemplate.seed.skill_bindings.map((skill) => skill.id))
    : presetCapabilities.skillNames;
  const presetSkillNamesKey = Array.from(presetSkillNames).sort().join('\u0000');
  const requiresMcpResource = presetResourceKinds.has('mcp_server');
  const frozenMcpTools = hasFrozenMcpTools(presetCapabilityIds);
  const requiredMcpServerIds = useMemo(() => requiresMcpResource
    ? selectedMcpResourceIds(resourceSelectionValue) : [],
  [requiresMcpResource, resourceSelectionValue]);
  const effectiveCapabilityDraft = useMemo<SessionCapabilityDraft>(() => ({
    skillNames: capabilityDraft.skillNames,
    mcpServerIds: frozenMcpTools ? requiredMcpServerIds
      : Array.from(new Set([...capabilityDraft.mcpServerIds, ...requiredMcpServerIds])),
  }), [capabilityDraft, frozenMcpTools, requiredMcpServerIds]);
  const lockedMcpServerIds = useMemo(
    () => new Set(requiredMcpServerIds),
    [requiredMcpServerIds]
  );
  const displayedCapabilityCatalog = useMemo(() => frozenMcpTools ? {
    ...capabilityCatalog.catalog,
    mcpServers: capabilityCatalog.catalog.mcpServers.filter((server) => lockedMcpServerIds.has(server.mcp_server_id)),
  } : capabilityCatalog.catalog, [capabilityCatalog.catalog, frozenMcpTools, lockedMcpServerIds]);
  const handleCapabilityDraftChange = useCallback((next: SessionCapabilityDraft) => {
    setCapabilityDraft({
      skillNames: next.skillNames,
      mcpServerIds: frozenMcpTools ? [] : next.mcpServerIds.filter((id) => !lockedMcpServerIds.has(id)),
    });
  }, [frozenMcpTools, lockedMcpServerIds]);

  useEffect(() => {
    if (capabilityCatalog.loading || capabilityCatalog.error) return;
    setCapabilityDraft(
      defaultSessionCapabilityDraft(capabilityCatalog.catalog, presetSkillNames, advancedControlsEnabled)
    );
  }, [
    capabilityCatalog.catalog,
    capabilityCatalog.error,
    capabilityCatalog.loading,
    presetSkillNamesKey,
    advancedControlsEnabled,
    selectedAgentResourceKey,
  ]);

  const capabilitySelection = buildSessionCapabilitySelection(
    effectiveCapabilityDraft,
    capabilityCatalog.catalog.autoSkillNames
  );
  const capabilitySelectionReady = !capabilityCatalog.loading && !capabilityCatalog.error;

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
        allowAutomation: advancedControlsEnabled,
      }),
    autoWork: effectiveAutoWork,
    workspaceEnabled,
    resourceResolutionReady: resourceSelectionsReady && (isCompanionAgent || collaboration.ready),
    collaboration: collaboration.config,
    resourceSelections: resourceSelectionResolution.selections,
    capabilitySelection: capabilitySelectionReady ? capabilitySelection : undefined,
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
      mention.setMentionOpen(false);
      mention.setMentionQuery(null);
      mention.setMentionSelectorOpen(false);
      mention.setMentionActiveIndex(0);
    },
    [
      agentSelection.setSelection,
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
    if (!resetAgentRequested) return;
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
  ]);

  useEffect(() => {
    if (!resetAgentRequested && !preselectedPresetId) return;
    if (
      preselectedPresetId &&
      (agentSelection.isLoading || !agentSelection.isLoaded)
    ) {
      return;
    }
    if (preselectedPresetId && agentSelection.loadError) return;
    const preselectionResolved =
      !preselectedPresetId ||
      (agentSelection.selection.kind === 'preset' &&
        agentSelection.selection.presetId === preselectedPresetId) ||
      !agentSelection.presets.some(
        (preset) => preset.preset_id === preselectedPresetId
      );
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
    agentSelection.presets,
    agentSelection.selection,
    preselectedPresetId,
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

  const advancedControlsNode = knowledgeEnabled || advancedControlsEnabled ? (
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
        <>
          <AutoWorkControl
            key={`autowork-${location.key}`}
            draft={{
              value: advancedConfig.autoWork,
              onChange: advancedConfig.setAutoWork,
            }}
            applyNote={t('guid.advanced.applyNote')}
          />
        </>
      )}
    </>
  ) : null;

  const modelSelectorNode = (
    creation.draft.mode ? <CreationModelSelector files={guidInput.files} /> : isCompanionAgent && selectedCompanion.profile?.model ? (
      <Button type='text' size='small' onClick={() => void navigate(`/nomi?companion=${selectedCompanion.profile!.companion_id}&tab=overview`)}>
        {selectedCompanion.profile.model.model}
      </Button>
    ) : (
      <ChatModelSelector
        providers={modelSelection.modelList}
        currentModel={modelSelection.current_model}
        getAvailableModels={modelSelection.getAvailableModels}
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

            <Composer
              sideTools={
                !isCompanionAgent && <SessionCapabilityPicker
                  catalog={displayedCapabilityCatalog}
                  draft={effectiveCapabilityDraft}
                  onChange={handleCapabilityDraftChange}
                  loading={capabilityCatalog.loading}
                  loadFailed={Boolean(capabilityCatalog.error)}
                  onRetry={capabilityCatalog.retry}
                  applyMode='create'
                  disabled={guidInput.loading}
                  lockedMcpServerIds={lockedMcpServerIds}
                >
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
                  />
                </SessionCapabilityPicker>
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
              footer={<>
                <SceneDiscoveryHint />
                {workspaceEnabled && <GuidWorkspaceFootnote workspaceDir={guidInput.dir} onSelectWorkspace={guidInput.setDir} onClearWorkspace={() => guidInput.setDir('')} />}
              </>}
            />

            {!creation.draft.mode && <AgentResourcePicker
              requiredKinds={resourcePickerKinds}
              companionBindings={isCompanionAgent}
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
