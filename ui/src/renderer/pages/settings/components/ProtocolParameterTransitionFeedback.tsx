import { Button } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { restoreCapabilityProtocolParams, suspendCapabilityTransitionIssues } from './chatProtocolDraftTransition';
import type { ModelCapabilityDraft } from './providerModelAdvanced';

interface Props {
  capability: ModelCapabilityDraft;
  onUpdate: (update: (current: ModelCapabilityDraft) => ModelCapabilityDraft) => void;
  onReturn: (protocol: string) => void;
}

/** Draft-only migration review; no stash or review metadata enters the API input. */
export default function ProtocolParameterTransitionFeedback({ capability, onUpdate, onReturn }: Props) {
  const { t } = useTranslation();
  const notice = capability.protocolParameterTransition;
  const canRestore = Object.keys(capability.protocolParameterStash?.[capability.protocol] ?? {}).length > 0;
  if (!notice && !canRestore) return null;
  const label = (parameter: string): string => {
    switch (parameter) {
      case 'response_format': case 'text.format': return t('settings.protocolTransition.outputFormat');
      case 'verbosity': case 'text.verbosity': return t('settings.protocolTransition.verbosity');
      case 'reasoning': case 'reasoning.effort': case 'reasoning_effort': return t('settings.protocolTransition.reasoning');
      case 'stream_options': return t('settings.protocolTransition.streamUsage');
      case 'max_tokens_field': return t('settings.protocolTransition.outputParameter');
      case 'require_reasoning_content': return t('settings.protocolTransition.reasoningContent');
      case 'chain_rounds': return t('settings.protocolTransition.chainRounds');
      case 'include': return t('settings.protocolTransition.responseExtras');
      case 'text': return t('settings.protocolTransition.answerOptions');
      default: return parameter;
    }
  };
  const labels = (keys: string[]) => [...new Set(keys.map(label))].join('、');
  return <div className='space-y-6px rounded-8px bg-fill-2 p-10px text-11px leading-18px' data-protocol-parameter-transition>
    {notice?.invalidJson ? <>
      <div role='alert'>{t('settings.protocolTransition.invalidJson')}</div>
      <Button size='small' onClick={() => onReturn(notice.source)}>{t('settings.protocolTransition.returnProtocol', { protocol: notice.source })}</Button>
    </> : <>
      {!!notice?.mappedKeys.length && <div>{t('settings.protocolTransition.mapped', { settings: labels(notice.mappedKeys) })}</div>}
      {!!notice?.suspendedKeys.length && <div>{t('settings.protocolTransition.suspended', { settings: labels(notice.suspendedKeys) })}</div>}
      {!!notice?.restoredKeys.length && <div>{t('settings.protocolTransition.restored', { settings: labels(notice.restoredKeys) })}</div>}
      {!!notice?.issues.length && <div className='space-y-6px text-t-primary border-l-2px border-l-solid border-warning-6 pl-8px' role='alert' data-protocol-transition-conflicts>
        <div>{t('settings.protocolTransition.needsChoice', { settings: labels(notice.issues.map(issue => issue.parameter)) })}</div>
        <div>{t('settings.protocolTransition.choiceImpact')}</div>
        <Button size='small' onClick={() => onUpdate(suspendCapabilityTransitionIssues)}>{t('settings.protocolTransition.useTarget')}</Button>
      </div>}
      {canRestore && <div className='space-y-6px' data-protocol-transition-restore>
        <div>{t('settings.protocolTransition.restoreImpact')}</div>
        <Button size='small' onClick={() => onUpdate(restoreCapabilityProtocolParams)}>{t('settings.protocolTransition.restore')}</Button>
      </div>}
    </>}
    <div className='text-t-secondary'>{t('settings.protocolTransition.draftOnly')}</div>
  </div>;
}
