import { useTranslation } from 'react-i18next';
import GuidCollaboratorSelector, { type GuidCollaboratorSelectorProps } from '@/renderer/pages/guid/components/GuidCollaboratorSelector';
import CollaborationPolicyControl, { type CollaborationPolicyValue } from './CollaborationPolicyControl';

/** Identical collaboration entry for a new draft and an existing conversation. */
export default function CollaborationComposerControl({ policy, onPolicyChange, runtimeType = 'nomi', ...models }: Omit<GuidCollaboratorSelectorProps, 'panelFooter' | 'triggerLabel' | 'triggerActive' | 'className'> & {
  policy: CollaborationPolicyValue;
  onPolicyChange: (policy: CollaborationPolicyValue) => void | Promise<void>;
  runtimeType?: string;
}) {
  const { t } = useTranslation();
  return <GuidCollaboratorSelector {...models}
    className='nomi-sendbox-model-btn nomi-sendbox-collaboration-btn'
    triggerLabel={t('collaboration.policy.button', { defaultValue: 'Collaboration' })}
    triggerActive={policy.delegationPolicy !== 'disabled'}
    panelFooter={<CollaborationPolicyControl runtimeType={runtimeType}
      delegationPolicy={policy.delegationPolicy} decisionPolicy={policy.decisionPolicy}
      onChange={onPolicyChange} embedded />}
  />;
}
