/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useConfig } from '@/renderer/hooks/config/useConfig';
import { useInputFocusRing } from '@/renderer/hooks/chat/useInputFocusRing';
import { isSubmitGesture } from '@/renderer/hooks/chat/useCompositionInput';
import { appendSpeechTranscript } from '@/renderer/hooks/system/useSpeechInput';
import SpeechInputButton from '@/renderer/components/chat/SpeechInputButton';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import { usePendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
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
import GuidActionRow from './components/GuidActionRow';
import GuidCompanionPosterPreview from './components/GuidCompanionPosterPreview';
import GuidInputCard from './components/GuidInputCard';
import GuidModelSelector from './components/GuidModelSelector';
import GuidResourceCards from './components/GuidResourceCards';
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
import { useGuidInput } from './hooks/useGuidInput';
import { useGuidMention } from './hooks/useGuidMention';
import { useGuidModelSelection } from './hooks/useGuidModelSelection';
import { useGuidPresetCapabilities } from './hooks/useGuidPresetCapabilities';
import { useGuidSend } from './hooks/useGuidSend';
import { useTypewriterPlaceholder } from './hooks/useTypewriterPlaceholder';
import type { GuidAgentSelection } from './types';
import styles from './index.module.css';

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
  const { activeBorderColor, inactiveBorderColor, activeShadow } =
    useInputFocusRing();
  const [showFeedbackModal, setShowFeedbackModal] = useState(false);

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
  const guidInput = useGuidInput({
    locationState: navigationState,
  });
  const advancedConfig = useGuidAdvancedConfig();
  const presetCapabilities = useGuidPresetCapabilities(
    agentSelection.selection.kind === 'preset'
      ? agentSelection.selection.presetId
      : undefined
  );

  const isAutoWorkMode = isAutoWorkEntry(advancedConfig.autoWork);
  const presetResourceResolutionReady = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
    : !presetCapabilities.isLoading && !presetCapabilities.error;
  const presetResourceKinds = agentSelection.selectedTemplate
    ? new Set(agentSelection.selectedTemplate.seed.required_resource_kinds)
    : presetCapabilities.requiredResourceKinds;
  const knowledgeEnabled =
    presetResourceResolutionReady && presetResourceKinds.has('knowledge_base');
  const workspaceEnabled =
    presetResourceResolutionReady && presetResourceKinds.has('workspace');
  const hasAgentLaunchTarget = agentSelection.selection.kind === 'template'
    ? Boolean(agentSelection.selectedTemplate)
    : Boolean(
        agentSelection.selectedPreset?.current_stable_revision &&
          presetResourceResolutionReady
      );
  const hasLaunchTarget = hasAgentLaunchTarget && Boolean(modelSelection.current_model);

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
      }),
    autoWork: advancedConfig.autoWork,
    workspaceEnabled,
    resourceResolutionReady: presetResourceResolutionReady,
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
        send.sendMessageHandler();
      }
    },
    [
      guidInput.input,
      isAutoWorkMode,
      mention,
      sendKey,
      send.sendMessageHandler,
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
    guidInput.setInput('');
    guidInput.setFiles([]);
    guidInput.setLoading(false);
    if (!navigationState?.workspace) {
      guidInput.setDir('');
    }
    advancedConfig.reset();
  }, [
    advancedConfig.reset,
    guidInput.setDir,
    guidInput.setFiles,
    guidInput.setInput,
    guidInput.setLoading,
    location.key,
    navigationState?.workspace,
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

  const advancedControlsNode = (
    <>
      <AutoWorkControl
        key={`autowork-${location.key}`}
        draft={{
          value: advancedConfig.autoWork,
          onChange: advancedConfig.setAutoWork,
        }}
        applyNote={t('guid.advanced.applyNote')}
      />
      <IdmmControl
        key={`idmm-${location.key}`}
        draft={{
          value: advancedConfig.idmm,
          onChange: advancedConfig.setIdmm,
        }}
        applyNote={t('guid.advanced.applyNote')}
      />
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
    </>
  );

  const modelSelectorNode = (
    <GuidModelSelector
      isProviderModelMode
      modelList={modelSelection.modelList}
      current_model={modelSelection.current_model}
      setCurrentModel={modelSelection.setCurrentModel}
    />
  );

  const autoWorkButtonDisabled =
    !hasLaunchTarget ||
    autoWorkStartDisabled(guidInput.loading, advancedConfig.autoWork);
  const actionRowNode = (
    <GuidActionRow
      files={guidInput.files}
      onFilesUploaded={guidInput.handleFilesUploaded}
      modelSelectorNode={modelSelectorNode}
      loading={guidInput.loading}
      speechInputNode={
        <SpeechInputButton
          disabled={guidInput.loading}
          locale={i18n.language}
          onTranscript={(transcript) => {
            guidInput.setInput((current) =>
              appendSpeechTranscript(current, transcript)
            );
          }}
        />
      }
      autoWorkMode={isAutoWorkMode}
      isButtonDisabled={
        isAutoWorkMode ? autoWorkButtonDisabled : send.isButtonDisabled
      }
      onSend={send.sendMessageHandler}
    />
  );

  return (
    <ConfigProvider
      getPopupContainer={() => guidContainerRef.current || document.body}
    >
      <div ref={guidContainerRef} className={styles.guidContainer}>
        <div className={styles.guidAdvancedControls}>
          {advancedControlsNode}
        </div>
        <div className={styles.guidPrimaryStage}>
          <div className={styles.guidLayout}>
            <div className={styles.heroHeader}>
              <p className='text-2xl font-semibold mb-0 text-0 text-center'>
                {t('conversation.welcome.title')}
              </p>
            </div>

            {agentSelection.selection.kind === 'preset' && presetCapabilities.error && (
              <Alert
                type='error'
                showIcon
                title={t('common.error')}
                content={t('agentSettings.errors.presetCapabilitiesLoadFailed')}
                className={styles.guidPresetCapabilityError}
              />
            )}

            <GuidInputCard
              input={guidInput.input}
              onInputChange={handleInputChange}
              onKeyDown={handleInputKeyDown}
              onPaste={guidInput.onPaste}
              onFocus={guidInput.handleTextareaFocus}
              onBlur={guidInput.handleTextareaBlur}
              placeholder={normalPlaceholder}
              isInputActive={guidInput.isInputFocused}
              isFileDragging={guidInput.isFileDragging}
              activeBorderColor={activeBorderColor}
              inactiveBorderColor={inactiveBorderColor}
              activeShadow={activeShadow}
              dragHandlers={guidInput.dragHandlers}
              mentionOpen={mention.mentionOpen}
              mentionSelectorBadge={
                <MentionSelectorBadge
                  visible={mention.mentionSelectorVisible}
                  open={mention.mentionSelectorOpen}
                  onOpenChange={mention.setMentionSelectorOpen}
                  agentLabel={mention.selectedAgentLabel}
                  mentionMenu={mentionDropdownNode}
                  onResetQuery={() => mention.setMentionQuery(null)}
                />
              }
              mentionDropdown={mentionDropdownNode}
              files={guidInput.files}
              onRemoveFile={guidInput.handleRemoveFile}
              actionRow={actionRowNode}
              showWorkspace={workspaceEnabled}
              workspaceDir={guidInput.dir}
              onSelectWorkspace={guidInput.setDir}
              onClearWorkspace={() => guidInput.setDir('')}
              agentSelector={
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
              }
            />

            <GuidResourceCards />
          </div>
        </div>

        <div className={styles.guidDiscoveryArea}>
          <GuidCompanionPosterPreview />
        </div>

        <QuickActionButtons
          onOpenBugReport={() => setShowFeedbackModal(true)}
          inactiveBorderColor={inactiveBorderColor}
          activeShadow={activeShadow}
        />
        <FeedbackReportModal
          visible={showFeedbackModal}
          onCancel={() => setShowFeedbackModal(false)}
        />
      </div>
    </ConfigProvider>
  );
};

export default GuidPage;
