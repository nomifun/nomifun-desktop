import ChatModelSelector from '@/renderer/components/chat/ChatModelSelector';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { useTranslation } from 'react-i18next';
import type { NomiModelSelection } from './useNomiModelSelection';

/** Adapt conversation-owned selection to the same picker used on the home page. */
export default function NomiModelSelector({ selection, disabled = false, compact, className }: {
  selection?: NomiModelSelection;
  disabled?: boolean;
  compact?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const { isOpen } = usePreviewContext();
  return <ChatModelSelector providers={selection?.providers ?? []} currentModel={selection?.current_model}
    getAvailableModels={provider => selection?.getAvailableModels(provider) ?? []}
    onSelectModel={async (provider, model) => { await selection?.handleSelectModel(provider, model); }}
    disabled={disabled || !selection} compact={compact ?? isOpen} className={className}
    readOnlyLabel={!selection ? t('conversation.welcome.useCliModel') : undefined}
    testId='nomi-model-selector' />;
}
