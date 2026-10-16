import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetSummary,
  AgentPresetRecommendedCapabilitiesResponse,
  ChatRouteRecord,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
} from '@/common/types/agentPlatform';
import {
  Button,
  Input,
  Tag,
} from '@arco-design/web-react';
import {
  Edit,
  MessageOne,
  Save,
  User,
} from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import AgentContributionOrder from './AgentContributionOrder';
import { unavailableModuleReferences } from './capabilityGroups';
import {
  TEMPLATE_I18N_PATH,
  updateDocument,
} from './model';
import styles from './AgentSettingsPage.module.css';
import { AgentEditorActionBar, AgentEditorActionButton } from './AgentEditorActionBar';
import AgentEditorTabs from './AgentEditorTabs';
import AgentInlineNameEditor from './AgentInlineNameEditor';
import AgentRuntimePolicyPanel from './AgentRuntimePolicyPanel';
import AgentRecommendedCapabilities from './AgentRecommendedCapabilities';
import AgentDefaultModelPicker from './AgentDefaultModelPicker';
import { isCompanionAgentTemplateKey } from '@/renderer/components/agent/conversationAgentCatalog';

type AgentPresetEditorProps = {
  editor: AgentPresetEditorResponse;
  draft: AgentPresetDraft;
  catalog: AgentCatalogResponse;
  sourceTemplate?: OfficialPresetTemplate;
  busyAction:
    | 'save'
    | 'create'
    | 'open'
    | 'delete'
    | 'recommend'
    | null;
  dirty: boolean;
  recommendation?: AgentPresetRecommendedCapabilitiesResponse | null;
  onRecommend?: () => void;
  onApplyRecommendation?: () => void;
  onDismissRecommendation?: () => void;
  onDraftChange: (draft: AgentPresetDraft) => void;
  onSave: () => void;
  saveLabel?: string;
  onDiscard?: () => void;
  onOpenModels?: () => void;
  onCopy?: () => void;
  onStartConversation: (preset: AgentPresetSummary) => void;
  startConversationLabel?: string;
};

export const AgentConversationAction: React.FC<{
  hasStableRevision: boolean;
  dirty: boolean;
  busy?: boolean;
  onClick: () => void;
  label?: string;
}> = ({ hasStableRevision, dirty, busy = false, onClick, label }) => {
  const { t } = useTranslation();

  return (
    <AgentEditorActionButton
      icon={<MessageOne theme='outline' size={15} fill='currentColor' />}
      disabled={busy || !hasStableRevision || dirty}
      onClick={onClick}
    >
      {label ?? t('agentSettings.actions.startConversation')}
    </AgentEditorActionButton>
  );
};

const AgentPresetEditor: React.FC<AgentPresetEditorProps> = ({
  editor, draft, catalog, sourceTemplate, busyAction, dirty, onDraftChange,
  onSave, saveLabel, onDiscard, onOpenModels, onCopy, onStartConversation, startConversationLabel,
  recommendation, onRecommend, onApplyRecommendation, onDismissRecommendation,
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState('capabilities');
  const [modelPending, setModelPending] = useState(false);
  const busy = busyAction !== null;
  const unavailableCount = unavailableModuleReferences(draft.document, catalog).length;
  const chatRouteRecord = draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK];
  const idmmPolicy = draft.document.runtime_policy.idmm;
  const idmmPolicyBlocksSave = idmmPolicy.mode === 'rule_plus_model' &&
    (!idmmPolicy.bypass_model.provider_id || !idmmPolicy.bypass_model.model);
  const moduleIds = draft.document.enabled_capabilities.map((selection) => String(selection.capability.id));
  const companionCoreMissing = editor.preset.source === 'official' &&
    isCompanionAgentTemplateKey(draft.source_template_key) &&
    ['companion', 'companion.memory'].some(id => !moduleIds.includes(id));
  const patchDocument = (transform: Parameters<typeof updateDocument>[1]) => onDraftChange(updateDocument(draft, transform));
  const applyChatRouteRecord = (record: ChatRouteRecord | undefined) => patchDocument((document) => {
    const model_route_refs = { ...document.model_route_refs };
    const chat_route_records = { ...document.chat_route_records };
    if (record) {
      model_route_refs[AGENT_CHAT_MODEL_TASK] = record.primary.model_route_id;
      chat_route_records[AGENT_CHAT_MODEL_TASK] = record;
    } else {
      delete model_route_refs[AGENT_CHAT_MODEL_TASK];
      delete chat_route_records[AGENT_CHAT_MODEL_TASK];
    }
    return { ...document, model_route_refs, chat_route_records };
  });
  const tabs = [
    { key: 'capabilities', label: t('agentSettings.workbench.capabilityTab') },
    { key: 'providers', label: t('agentSettings.providers.title') },
    { key: 'settings', label: t('agentSettings.workbench.settingsTab') },
    { key: 'runtime', label: t('agentSettings.workbench.runtimeTab') },
    { key: 'extensions', label: t('agentSettings.workbench.extensionsTab') },
  ];
  const editIdentity = () => setActiveTab('settings');

  return <main className={styles.editorSurface}>
    <header className={styles.editorHeader}>
      <div className={styles.editorHeaderCopy}>
        <div className={styles.headerEyebrow}>{t(editor.preset.source === 'official' ? 'agentSettings.workbench.officialBadge' : 'agentSettings.workbench.personalBadge')}</div>
        <AgentInlineNameEditor
          value={draft.display_name}
          fallback={t('agentSettings.defaults.untitledName')}
          disabled={busy}
          onChange={(displayName) => onDraftChange({ ...draft, display_name: displayName })}
        />
        <p>{draft.description || t('agentSettings.workbench.selectedHint')}</p>
      </div>
      <div className={styles.editorHeaderActions}>
        {sourceTemplate && <Tag size='small'>{t(`agentSettings.template.${TEMPLATE_I18N_PATH[sourceTemplate.template_key]}.name`)}</Tag>}
        {onCopy && <Button type='text' size='small' disabled={busy} onClick={onCopy}>{t('agentSettings.workbench.createCopy')}</Button>}
        {editor.preset.source === 'official' && sourceTemplate && onRecommend && <Button type='text' size='small'
          disabled={busy || dirty || !editor.preset.current_stable_revision}
          loading={busyAction === 'recommend'}
          title={dirty ? t('agentSettings.recommendation.saveFirst') : undefined}
          onClick={onRecommend}>{t('agentSettings.recommendation.preview')}</Button>}
        <Button
          className={styles.identityEdit}
          type='text'
          size='small'
          icon={<Edit theme='outline' size={14} />}
          disabled={busy}
          aria-label={t('agentSettings.workbench.editIdentity')}
          title={t('agentSettings.workbench.editIdentity')}
          aria-controls='agent-panel-settings'
          onClick={editIdentity}
        >
          {t('agentSettings.workbench.editIdentity')}
        </Button>
      </div>
    </header>
    <AgentEditorTabs tabs={tabs} active={activeTab} onChange={setActiveTab} idPrefix='agent' label={t('agentSettings.title')} />
    <div className={`${styles.editorBody} ${activeTab === 'capabilities' ? styles.capabilityBody : ''} ${activeTab === 'settings' ? styles.identityBody : ''}`}>
      {recommendation && onApplyRecommendation && onDismissRecommendation && <AgentRecommendedCapabilities
        recommendation={recommendation} catalog={catalog} busy={busy || dirty} saving={busyAction === 'save'}
        onApply={onApplyRecommendation} onDismiss={onDismissRecommendation} />}
      {activeTab === 'capabilities' && <div className={styles.capabilityPanel} role='tabpanel' id='agent-panel-capabilities' aria-labelledby='agent-tab-capabilities'>
        <AgentCapabilityWorkspace document={draft.document} catalog={catalog} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
      {activeTab === 'providers' && <div role='tabpanel' id='agent-panel-providers' aria-labelledby='agent-tab-providers'>
        <AgentRoleProviderPicker document={draft.document} catalog={catalog} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
      <div className={styles.identityPanel} hidden={activeTab !== 'settings'} role='tabpanel' id='agent-panel-settings' aria-labelledby='agent-tab-settings'>
        <section className={styles.identityCard} id='agent-settings-basic' aria-labelledby='agent-identity-heading'>
          <div className={styles.identityHeading}>
            <span className={styles.identitySectionIcon} aria-hidden='true'><User theme='outline' size={18} /></span>
            <div>
              <h3 id='agent-identity-heading'>{t('agentSettings.sections.basic')}</h3>
              <p>{t('agentSettings.sections.basicHint')}</p>
            </div>
          </div>
          <div className={styles.identityFields}>
            <label className={styles.field}>
              <span>{t('agentSettings.fields.name')}</span>
              <Input
                value={draft.display_name}
                placeholder={t('agentSettings.fields.namePlaceholder')}
                maxLength={80}
                disabled={busy}
                onChange={(displayName: string) => onDraftChange({ ...draft, display_name: displayName })}
                onInput={(event) => onDraftChange({ ...draft, display_name: (event.target as HTMLInputElement).value })}
              />
            </label>
            <div className={styles.field}>
              <div className={styles.modelFieldHeading}>
                <span>{t('agentSettings.fields.defaultModel')}<small>{t('common.optional')}</small></span>
                {onOpenModels && <Button className={styles.modelManagement} type='text' size='mini' disabled={busy} onClick={onOpenModels}>
                  {t('agentSettings.workbench.manageModels')}
                </Button>}
              </div>
              <AgentDefaultModelPicker
                presetId={draft.preset_id}
                record={chatRouteRecord}
                disabled={busy}
                onChange={applyChatRouteRecord}
                onPendingChange={setModelPending}
              />
            </div>
            <label className={`${styles.field} ${styles.fieldWide}`}>
              <span>{t('agentSettings.fields.description')}</span>
              <Input
                value={draft.description ?? ''}
                placeholder={t('agentSettings.fields.descriptionPlaceholder')}
                maxLength={240}
                disabled={busy}
                onChange={(description: string) => onDraftChange({ ...draft, description: description || undefined })}
              />
            </label>
          </div>
        </section>
        <section className={styles.identityCard} aria-labelledby='agent-behavior-heading'>
          <div className={styles.identityHeading}>
            <span className={styles.identitySectionIcon} aria-hidden='true'><Edit theme='outline' size={18} /></span>
            <div>
              <h3 id='agent-behavior-heading'>{t('agentSettings.sections.behavior')}</h3>
              <p>{t('agentSettings.sections.behaviorHint')}</p>
            </div>
          </div>
          <div className={styles.writingFields}>
            <label className={styles.field}>
              <span>{t('agentSettings.fields.persona')}</span>
              <Input.TextArea
                value={draft.document.persona}
                placeholder={t('agentSettings.fields.personaPlaceholder')}
                autoSize={{ minRows: 4, maxRows: 10 }}
                disabled={busy}
                onChange={(persona: string) => patchDocument((document) => ({ ...document, persona }))}
              />
            </label>
            <label className={styles.field}>
              <span>{t('agentSettings.fields.instructions')}</span>
              <Input.TextArea
                value={draft.document.instructions}
                placeholder={t('agentSettings.fields.instructionsPlaceholder')}
                autoSize={{ minRows: 4, maxRows: 10 }}
                disabled={busy}
                onChange={(instructions: string) => patchDocument((document) => ({ ...document, instructions }))}
              />
            </label>
          </div>
        </section>
      </div>
      {activeTab === 'runtime' && <AgentRuntimePolicyPanel idPrefix='agent' document={draft.document} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />}
      {activeTab === 'extensions' && <div role='tabpanel' id='agent-panel-extensions' aria-labelledby='agent-tab-extensions'>
        <AgentContributionOrder kind='middleware' document={draft.document} catalog={catalog.capabilities} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
        <AgentContributionOrder document={draft.document} catalog={catalog.capabilities} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
    </div>
    <AgentEditorActionBar>
      <div className={`${styles.saveStatus} ${companionCoreMissing ? styles.saveStatusWarning : ''}`}><span className={dirty || unavailableCount || idmmPolicyBlocksSave || companionCoreMissing ? styles.statusWarningDot : styles.statusReadyDot} /><div><strong>{t(dirty ? 'agentSettings.workbench.pendingChanges' : 'agentSettings.workbench.savedHint')}</strong><span>{t(companionCoreMissing ? 'agentSettings.workbench.companionCoreNeeded' : unavailableCount ? 'agentSettings.workbench.disabledSave' : idmmPolicyBlocksSave ? 'agentSettings.workbench.runtimePolicyNeeded' : editor.preset.source === 'official' ? 'agentSettings.workbench.officialSaveHint' : dirty ? 'agentSettings.workbench.previewCompileHint' : 'agentSettings.workbench.saveHint')}</span></div></div>
      <div className={styles.actionButtons}>
        {dirty && onDiscard && <Button type='text' disabled={busy} onClick={onDiscard}>{t('agentSettings.workbench.resetChanges')}</Button>}
        {dirty || !editor.preset.current_stable_revision ? <AgentEditorActionButton icon={<Save theme='outline' size={15} fill='currentColor' />} loading={busyAction === 'save'} disabled={busy || unavailableCount > 0 || !draft.display_name.trim() || modelPending || idmmPolicyBlocksSave || companionCoreMissing} onClick={onSave}>{saveLabel ?? t('common.save')}</AgentEditorActionButton> :
          <AgentConversationAction hasStableRevision={Boolean(editor.preset.current_stable_revision)} dirty={dirty} busy={busy || modelPending} label={startConversationLabel} onClick={() => onStartConversation(editor.preset)} />}
      </div>
    </AgentEditorActionBar>
  </main>;
};

export default AgentPresetEditor;
