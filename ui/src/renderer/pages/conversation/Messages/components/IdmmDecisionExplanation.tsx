import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { IdmmDecisionExplanation as Decision } from '@/common/types/idmm';
import { parseMessageId, type ConversationId } from '@/common/types/ids';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { dispatchChatMessageJump } from '@/renderer/utils/chat/chatMinimapEvents';
import styles from './IdmmDecisionExplanation.module.css';

/** Only deterministic backend reason codes have localized fixed wording. */
const FIXED_BASIS_KEYS = {
  rule_selected_recommended_option: 'idmm.explanation.basis.recommendedOption',
  rule_selected_first_safe_option: 'idmm.explanation.basis.firstSafeOption',
  rule_selected_safe_option: 'idmm.explanation.basis.safeOption',
  provider_fault_detected: 'idmm.explanation.basis.providerRecovery',
  model_stalled: 'idmm.explanation.basis.modelRecovery',
  tool_stalled_safety_halt: 'idmm.explanation.basis.stalledTool',
  recovery_limit_reached: 'idmm.explanation.basis.recoveryLimit',
  sensitive_input_required: 'idmm.explanation.basis.sensitiveInput',
  destructive_answer_rejected: 'idmm.explanation.basis.destructiveAnswer',
  rule_cannot_answer: 'idmm.explanation.basis.unresolvedQuestion',
  bypass_model_failed: 'idmm.explanation.basis.bypassFailed',
} as const;

/** Presentation metadata stays outside the reply body and its copy/edit actions. */
export const IdmmDecisionExplanation = ({ decision, conversationId }: {
  decision: Decision; conversationId: ConversationId;
}) => {
  const { t } = useTranslation();
  const [showBasis] = useConfig('chat.idmm.showDecisionBasis');
  const [manuallyExpanded, setManuallyExpanded] = useState<boolean | null>(null);
  const expanded = manuallyExpanded ?? showBasis === true;
  const detailsId = useId();
  const source = decision.source === 'rule' ? t('idmm.explanation.ruleSource')
    : decision.source === 'recovery' ? t('idmm.explanation.recoverySource')
    : decision.model ? t('idmm.explanation.modelSource', { model: decision.model.model })
    : t('idmm.explanation.bypassSource');
  const fixedBasisKey = Object.hasOwn(FIXED_BASIS_KEYS, decision.reason_code)
    ? FIXED_BASIS_KEYS[decision.reason_code as keyof typeof FIXED_BASIS_KEYS] : undefined;
  const rationale = fixedBasisKey ? t(fixedBasisKey) : decision.rationale;
  return (
    <div className={styles.explanation} data-testid='idmm-decision-explanation' role='note'>
      <div className={styles.header}>
        <span className={styles.source}><span>{t('idmm.title')}</span><span aria-hidden='true'> · </span><span>{source}</span></span>
        <button type='button' className={styles.button} aria-expanded={expanded} aria-controls={detailsId}
          onClick={() => setManuallyExpanded(!expanded)}>
          {t(expanded ? 'idmm.explanation.hideBasis' : 'idmm.explanation.showBasis')}
        </button>
        {decision.question && <button type='button' className={styles.button} onClick={() => {
          dispatchChatMessageJump({ conversation_id: conversationId,
            messageId: parseMessageId(decision.question!.message_id), align: 'center', loadOlder: true });
        }}>{t('idmm.explanation.viewQuestion')}</button>}
      </div>
      <div id={detailsId} hidden={!expanded} className={styles.basis} data-testid='idmm-decision-basis'>{rationale}</div>
    </div>
  );
};
