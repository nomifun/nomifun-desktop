import ChatModelSelector from '@/renderer/components/chat/ChatModelSelector';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { useTranslation } from 'react-i18next';
import type { NomiModelSelection } from './useNomiModelSelection';
import {
  capabilityOf,
  capabilitySupportsTechnicalCapability,
} from '@/common/utils/providerModels';
import {
  reasoningEffortsForProtocol,
  type SessionReasoningEffort,
} from '@/common/types/reasoningEffort';

/** Adapt conversation-owned selection to the same picker used on the home page. */
export default function NomiModelSelector({
  selection,
  disabled = false,
  compact,
  className,
  reasoningEffort,
  reasoningEffortDisabled = false,
  onReasoningEffortChange,
}: {
  selection?: NomiModelSelection;
  disabled?: boolean;
  compact?: boolean;
  className?: string;
  reasoningEffort?: SessionReasoningEffort;
  reasoningEffortDisabled?: boolean;
  onReasoningEffortChange?: (value: SessionReasoningEffort | undefined) => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const { isOpen } = usePreviewContext();
  const currentCapability = selection?.current_model
    ? capabilityOf(
        selection.providers.find(provider => provider.id === selection.current_model?.id),
        selection.current_model.use_model,
        'chat'
      )
    : undefined;
  const reasoningEffortOptions = capabilitySupportsTechnicalCapability(
    currentCapability,
    'reasoning'
  )
    ? reasoningEffortsForProtocol(currentCapability?.protocol)
    : [];
  return <ChatModelSelector providers={selection?.providers ?? []} currentModel={selection?.current_model}
    getAvailableModels={provider => selection?.getAvailableModels(provider) ?? []}
    onSelectModel={async (provider, model) => { await selection?.handleSelectModel(provider, model); }}
    disabled={disabled || !selection || selection.pickerDisabled} compact={compact ?? isOpen} className={className}
    readOnlyLabel={!selection ? t('conversation.welcome.useCliModel') : undefined}
    reasoningEffort={reasoningEffort}
    reasoningEffortOptions={reasoningEffortOptions}
    reasoningEffortDisabled={reasoningEffortDisabled}
    onReasoningEffortChange={onReasoningEffortChange}
    testId='nomi-model-selector' />;
}
