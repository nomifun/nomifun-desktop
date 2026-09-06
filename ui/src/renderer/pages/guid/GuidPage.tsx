/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useConfig } from '@/renderer/hooks/config/useConfig';
import { useInputFocusRing } from '@/renderer/hooks/chat/useInputFocusRing';
import { isSubmitGesture } from '@/renderer/hooks/chat/useCompositionInput';
import { appendSpeechTranscript } from '@/renderer/hooks/system/useSpeechInput';
import { useMiniAppQuickStart } from '@/renderer/hooks/agent/useMiniAppQuickStart';
import SpeechInputButton from '@/renderer/components/chat/SpeechInputButton';
import FeedbackReportModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import { usePendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
import {
  SummonDrawer,
  useCompanionRoster,
} from '@/renderer/pages/conversation/components/SummonPanel';
import { ConfigProvider } from '@arco-design/web-react';
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
import AgentPillBar from './components/AgentPillBar';
import ComposerEntryStrip from './components/ComposerEntryStrip';
import { AgentPillBarSkeleton } from './components/GuidSkeleton';
import GuidActionRow from './components/GuidActionRow';
import GuidCompanionPosterPreview from './components/GuidCompanionPosterPreview';
import GuidInputCard from './components/GuidInputCard';
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
import { useGuidSend } from './hooks/useGuidSend';
import { useTypewriterPlaceholder } from './hooks/useTypewriterPlaceholder';
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
  const guidInput = useGuidInput({
    locationState: navigationState,
  });
  const advancedConfig = useGuidAdvancedConfig();

  const [miniAppMode, setMiniAppMode] = useState(false);
  const miniAppQuickStart = useMiniAppQuickStart();
  const miniAppSendingRef = useRef(false);

  const isAutoWorkMode = isAutoWorkEntry(advancedConfig.autoWork);
  const hasExecutablePreset = Boolean(
    agentSelection.selectedPreset?.current_stable_revision
  );

  useEffect(() => {
    if (isAutoWorkMode) setMiniAppMode(false);
  }, [isAutoWorkMode]);

  const [summonDrawerOpen, setSummonDrawerOpen] = useState(false);
  const companionRoster = useCompanionRoster();
  const summonedCompanionName = advancedConfig.summon
    ? companionRoster.find(
        (companion) =>
          companion.companion_id === advancedConfig.summon?.companion_id
      )?.name ?? null
    : null;

  const mention = useGuidMention({
    presets: agentSelection.presets,
    selectedPresetId: agentSelection.selectedPresetId,
    setSelectedPresetId: agentSelection.setSelectedPresetId,
    selectedPreset: agentSelection.selectedPreset,
    setInput: guidInput.setInput,
  });

  const send = useGuidSend({
    input: guidInput.input,
    setInput: guidInput.setInput,
    files: guidInput.files,
    setFiles: guidInput.setFiles,
    setDir: guidInput.setDir,
    setLoading: guidInput.setLoading,
    loading: guidInput.loading,
    selectedPreset: agentSelection.selectedPreset,
    applyAdvancedConfig: advancedConfig.applyToConversation,
    autoWork: advancedConfig.autoWork,
    setMentionOpen: mention.setMentionOpen,
    setMentionQuery: mention.setMentionQuery,
    setMentionSelectorOpen: mention.setMentionSelectorOpen,
    setMentionActiveIndex: mention.setMentionActiveIndex,
    navigate,
    t,
    beginPending: pendingConversation.begin,
    endPending: pendingConversation.end,
  });

  const handleComposerSend = useCallback(() => {
    if (!miniAppMode || isAutoWorkMode) {
      send.sendMessageHandler();
      return;
    }

    const prompt = guidInput.input.trim();
    if (!prompt || guidInput.loading || miniAppSendingRef.current) return;

    miniAppSendingRef.current = true;
    guidInput.setLoading(true);
    pendingConversation.begin({
      input: guidInput.input,
      files: guidInput.files.length > 0 ? guidInput.files : undefined,
      sendsInitialMessage: true,
    });

    void miniAppQuickStart
      .start({
        prompt,
        dir: guidInput.dir,
        files: guidInput.files,
      })
      .then((started) => {
        if (!started) return;
        guidInput.setInput('');
        guidInput.setFiles([]);
        guidInput.setDir('');
        mention.setMentionOpen(false);
        mention.setMentionQuery(null);
        mention.setMentionSelectorOpen(false);
        mention.setMentionActiveIndex(0);
        setMiniAppMode(false);
      })
      .finally(() => {
        miniAppSendingRef.current = false;
        guidInput.setLoading(false);
        pendingConversation.end();
      });
  }, [
    guidInput.dir,
    guidInput.files,
    guidInput.input,
    guidInput.loading,
    guidInput.setDir,
    guidInput.setFiles,
    guidInput.setInput,
    guidInput.setLoading,
    isAutoWorkMode,
    mention.setMentionActiveIndex,
    mention.setMentionOpen,
    mention.setMentionQuery,
    mention.setMentionSelectorOpen,
    miniAppMode,
    miniAppQuickStart.start,
    pendingConversation,
    send.sendMessageHandler,
  ]);

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
        handleComposerSend();
      }
    },
    [
      guidInput.input,
      handleComposerSend,
      isAutoWorkMode,
      mention,
      sendKey,
    ]
  );

  const handleSelectPresetFromPillBar = useCallback(
    (presetId: string) => {
      agentSelection.setSelectedPresetId(presetId);
      mention.setMentionOpen(false);
      mention.setMentionQuery(null);
      mention.setMentionSelectorOpen(false);
      mention.setMentionActiveIndex(0);
    },
    [
      agentSelection.setSelectedPresetId,
      mention.setMentionActiveIndex,
      mention.setMentionOpen,
      mention.setMentionQuery,
      mention.setMentionSelectorOpen,
    ]
  );

  const typewriterPlaceholder = useTypewriterPlaceholder(
    t('conversation.welcome.placeholder')
  );
  const normalPlaceholder = mention.selectedAgentLabel
    ? `${mention.selectedAgentLabel}, ${
        typewriterPlaceholder || t('conversation.welcome.placeholder')
      }`
    : t('guid.agentPresetRequired', {
        defaultValue: 'Select an Agent from Agent Workbench to start',
      });

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
    const preselectedPresetUnavailable =
      Boolean(preselectedPresetId) &&
      !agentSelection.isLoading &&
      !agentSelection.presets.some(
        (preset) => preset.preset_id === preselectedPresetId
      );
    if (
      preselectedPresetId &&
      agentSelection.selectedPresetId !== preselectedPresetId &&
      !preselectedPresetUnavailable
    ) {
      return;
    }
    if (
      resetAgentRequested &&
      agentSelection.isLoading
    ) {
      return;
    }
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
    agentSelection.presets,
    agentSelection.selectedPresetId,
    preselectedPresetId,
    resetAgentRequested,
  ]);

  const miniAppQueryRequested = useMemo(
    () => new URLSearchParams(location.search).get('miniapp') === '1',
    [location.search]
  );
  useEffect(() => {
    if (!miniAppQueryRequested) return;
    setMiniAppMode(true);
    navigate(`${location.pathname}${location.hash}`, {
      replace: true,
      state: null,
    });
  }, [
    location.hash,
    location.pathname,
    miniAppQueryRequested,
    navigate,
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
    </>
  );

  const autoWorkButtonDisabled =
    !hasExecutablePreset ||
    autoWorkStartDisabled(guidInput.loading, advancedConfig.autoWork);
  const miniAppButtonDisabled =
    guidInput.loading ||
    !guidInput.input.trim() ||
    !miniAppQuickStart.canStart;
  const actionRowNode = (
    <GuidActionRow
      files={guidInput.files}
      onFilesUploaded={guidInput.handleFilesUploaded}
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
        miniAppMode
          ? miniAppButtonDisabled
          : isAutoWorkMode
            ? autoWorkButtonDisabled
            : send.isButtonDisabled
      }
      onSend={handleComposerSend}
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

            {agentSelection.isLoading ? (
              <AgentPillBarSkeleton />
            ) : (
              <AgentPillBar
                presets={agentSelection.presets}
                selectedPresetId={agentSelection.selectedPresetId}
                onSelectPreset={handleSelectPresetFromPillBar}
                suppressSelectionAnimation={resetAgentRequested}
              />
            )}

            <GuidInputCard
              input={guidInput.input}
              onInputChange={handleInputChange}
              onKeyDown={handleInputKeyDown}
              onPaste={guidInput.onPaste}
              onFocus={guidInput.handleTextareaFocus}
              onBlur={guidInput.handleTextareaBlur}
              placeholder={
                miniAppMode
                  ? t('miniApps.composer.placeholder')
                  : normalPlaceholder
              }
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
              showWorkspace={miniAppMode}
              workspaceDir={guidInput.dir}
              onSelectWorkspace={guidInput.setDir}
              onClearWorkspace={() => guidInput.setDir('')}
              entryStrip={
                <ComposerEntryStrip
                  onSummonCompanion={() => setSummonDrawerOpen(true)}
                  summonedCompanionName={summonedCompanionName}
                  onCreateMiniApp={
                    isAutoWorkMode ? undefined : () => setMiniAppMode(true)
                  }
                  miniAppActive={miniAppMode}
                  onDismissMiniApp={() => setMiniAppMode(false)}
                />
              }
            />

            <GuidResourceCards />

            <SummonDrawer
              visible={summonDrawerOpen}
              onCancel={() => setSummonDrawerOpen(false)}
              initial={advancedConfig.summon}
              onApply={(draft) => {
                advancedConfig.setSummon(draft);
                setSummonDrawerOpen(false);
              }}
              onRelease={() => {
                advancedConfig.setSummon(null);
                setSummonDrawerOpen(false);
              }}
            />
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
