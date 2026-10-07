import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetSummary,
  ChatRouteCandidate,
  ChatRouteRecord,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
} from '@/common/types/agentPlatform';
import {
  Alert,
  Button,
  Input,
  Select,
  Tag,
} from '@arco-design/web-react';
import {
  Edit,
  MessageOne,
  Save,
} from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import AgentContributionOrder from './AgentContributionOrder';
import { unavailableModuleReferences } from './capabilityGroups';
import {
  TEMPLATE_I18N_PATH,
  chatRouteCandidateKey,
  selectChatRouteCandidate,
  updateDocument,
} from './model';
import styles from './AgentSettingsPage.module.css';
import { AgentEditorActionBar, AgentEditorActionButton } from './AgentEditorActionBar';
import AgentEditorTabs from './AgentEditorTabs';
import AgentInlineNameEditor from './AgentInlineNameEditor';
import AgentRuntimePolicyPanel from './AgentRuntimePolicyPanel';

type AgentPresetEditorProps = {
  editor: AgentPresetEditorResponse;
  draft: AgentPresetDraft;
  catalog: AgentCatalogResponse;
  sourceTemplate?: OfficialPresetTemplate;
  busyAction:
    | 'save'
    | 'fork'
    | 'create'
    | 'open'
    | 'delete'
    | null;
  dirty: boolean;
  onDraftChange: (draft: AgentPresetDraft) => void;
  onSave: () => void;
  saveLabel?: string;
  onDiscard?: () => void;
  onOpenModels?: () => void;
  onOpenAuthor?: (destination: string) => void | Promise<void>;
  onStartConversation: (preset: AgentPresetSummary) => void;
};

export const AgentConversationAction: React.FC<{
  hasStableRevision: boolean;
  dirty: boolean;
  busy?: boolean;
  onClick: () => void;
}> = ({ hasStableRevision, dirty, busy = false, onClick }) => {
  const { t } = useTranslation();

  return (
    <AgentEditorActionButton
      icon={<MessageOne theme='outline' size={15} fill='currentColor' />}
      disabled={busy || !hasStableRevision || dirty}
      onClick={onClick}
    >
      {t('agentSettings.actions.startConversation')}
    </AgentEditorActionButton>
  );
};

const AgentChatModelPicker: React.FC<{
  record?: ChatRouteRecord;
  disabled: boolean;
  onChange: (record: ChatRouteRecord) => void;
}> = ({ record, disabled, onChange }) => {
  const { t } = useTranslation();
  const { data: providers } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const candidates = useMemo<ChatRouteCandidate[]>(
    () => (record ? [record.primary, ...record.failovers] : []),
    [record]
  );
  const selectedKey = record ? chatRouteCandidateKey(record.primary) : undefined;

  const labelFor = (candidate: ChatRouteCandidate): string => {
    const provider = providers?.find((item) => String(item.id) === candidate.provider_id);
    const providerName = provider ? providerLabel(provider) : '';
    const model = provider?.models.find((item) => item.model === candidate.model);
    const modelName = modelDisplayLabel(candidate.model, model?.display_name);
    return providerName ? `${providerName} / ${modelName}` : modelName;
  };

  return (
    <div className={styles.modelPicker}>
      <Select
        value={selectedKey}
        disabled={disabled || candidates.length === 0}
        placeholder={t('settings.taskModel.modelPlaceholder')}
        onChange={(next: string) => {
          const selected = selectChatRouteCandidate(record, next);
          if (selected) onChange(selected);
        }}
      >
        {candidates.map((candidate) => (
          <Select.Option
            key={chatRouteCandidateKey(candidate)}
            value={chatRouteCandidateKey(candidate)}
          >
            {labelFor(candidate)}
          </Select.Option>
        ))}
      </Select>
      {candidates.length === 0 && (
        <span className={styles.fieldHint}>{t('settings.taskModel.emptyHint')}</span>
      )}
    </div>
  );
};

const AgentPresetEditor: React.FC<AgentPresetEditorProps> = ({
  editor, draft, catalog, sourceTemplate, busyAction, dirty, onDraftChange,
  onSave, saveLabel, onDiscard, onOpenModels, onOpenAuthor, onStartConversation,
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState('capabilities');
  const busy = busyAction !== null;
  const unavailableCount = unavailableModuleReferences(draft.document, catalog).length;
  const chatRouteRecord = draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK];
  const idmmPolicy = draft.document.runtime_policy.idmm;
  const idmmPolicyBlocksSave = idmmPolicy.mode === 'rule_plus_model' &&
    (!idmmPolicy.bypass_model.provider_id || !idmmPolicy.bypass_model.model);
  const moduleIds = draft.document.enabled_capabilities.map((selection) => String(selection.capability.id));
  const taskOnlyCreation = moduleIds.includes('creation.media') && !chatRouteRecord;
  const savedDocumentUnchanged = Boolean(
    editor.revision && JSON.stringify(draft.document) === JSON.stringify(editor.revision.document)
  );
  const metadataOnlyChange = Boolean(editor.preset.current_stable_revision) && savedDocumentUnchanged;
  const needsChatModel = !taskOnlyCreation && !chatRouteRecord;
  const modelBlocksSave = needsChatModel && !metadataOnlyChange;
  const patchDocument = (transform: Parameters<typeof updateDocument>[1]) => onDraftChange(updateDocument(draft, transform));
  const applyChatRouteRecord = (record: ChatRouteRecord) => patchDocument((document) => ({
    ...document,
    model_route_refs: { ...document.model_route_refs, [AGENT_CHAT_MODEL_TASK]: record.primary.model_route_id },
    chat_route_records: { ...document.chat_route_records, [AGENT_CHAT_MODEL_TASK]: record },
  }));
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
        <div className={styles.headerEyebrow}>{t('agentSettings.workbench.personalBadge')}</div>
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
    <div className={`${styles.editorBody} ${activeTab === 'capabilities' ? styles.capabilityBody : ''}`}>
      {activeTab === 'capabilities' && <div className={styles.capabilityPanel} role='tabpanel' id='agent-panel-capabilities' aria-labelledby='agent-tab-capabilities'>
        <AgentCapabilityWorkspace document={draft.document} catalog={catalog} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
      {activeTab === 'providers' && <div role='tabpanel' id='agent-panel-providers' aria-labelledby='agent-tab-providers'>
        <AgentRoleProviderPicker document={draft.document} catalog={catalog} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
      {activeTab === 'settings' && <div role='tabpanel' id='agent-panel-settings' aria-labelledby='agent-tab-settings'>
        <section className={styles.section} id='agent-settings-basic'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.basic')}</h3>
            <p>{t('agentSettings.sections.basicHint')}</p>
          </div>
        </div>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>{t('agentSettings.fields.name')}</span>
            <Input
              value={draft.display_name}
              maxLength={80}
              disabled={busy}
              onChange={(displayName: string) =>
                onDraftChange({ ...draft, display_name: displayName })
              }
              onInput={(event) =>
                onDraftChange({ ...draft, display_name: (event.target as HTMLInputElement).value })
              }
            />
          </label>
          {!taskOnlyCreation && <label className={styles.field}>
            <span>{t('common.model')}</span>
            <AgentChatModelPicker
              record={chatRouteRecord}
              disabled={busy}
              onChange={applyChatRouteRecord}
            />
          </label>}
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.description')}</span>
            <Input
              value={draft.description ?? ''}
              maxLength={240}
              disabled={busy}
              onChange={(description: string) =>
                onDraftChange({
                  ...draft,
                  description: description || undefined,
                })
              }
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.persona')}</span>
            <Input.TextArea
              value={draft.document.persona}
              autoSize={{ minRows: 2, maxRows: 5 }}
              disabled={busy}
              onChange={(persona: string) =>
                patchDocument((document) => ({ ...document, persona }))
              }
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.instructions')}</span>
            <Input.TextArea
              value={draft.document.instructions}
              autoSize={{ minRows: 4, maxRows: 10 }}
              disabled={busy}
              onChange={(instructions: string) =>
                patchDocument((document) => ({ ...document, instructions }))
              }
            />
          </label>
        </div>
        {modelBlocksSave && (
          <Alert
            className={styles.inlineNotice}
            type='warning'
            showIcon
            content={t('agentSettings.fields.chatModelRouteUnavailable', {
              defaultValue:
                'This host did not provide an available Chat model route. Saving remains blocked until one is available.',
            })}
          />
        )}
      </section>
        {modelBlocksSave && onOpenModels && <Button type='text' onClick={onOpenModels}>{t('agentSettings.workbench.manageModels')}</Button>}
      </div>}
      {activeTab === 'runtime' && <AgentRuntimePolicyPanel idPrefix='agent' document={draft.document} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />}
      {activeTab === 'extensions' && <div role='tabpanel' id='agent-panel-extensions' aria-labelledby='agent-tab-extensions'>
        <AgentContributionOrder kind='middleware' document={draft.document} catalog={catalog.capabilities} disabled={busy} onOpenAuthor={onOpenAuthor} onChange={(document) => onDraftChange({ ...draft, document })} />
        <AgentContributionOrder document={draft.document} catalog={catalog.capabilities} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
      </div>}
    </div>
    <AgentEditorActionBar>
      <div className={styles.saveStatus}><span className={dirty || unavailableCount || idmmPolicyBlocksSave ? styles.statusWarningDot : styles.statusReadyDot} /><div><strong>{t(dirty ? 'agentSettings.workbench.pendingChanges' : 'agentSettings.workbench.savedHint')}</strong><span>{t(unavailableCount ? 'agentSettings.workbench.disabledSave' : idmmPolicyBlocksSave ? 'agentSettings.workbench.runtimePolicyNeeded' : modelBlocksSave ? 'agentSettings.workbench.modelNeeded' : dirty ? 'agentSettings.workbench.previewCompileHint' : 'agentSettings.workbench.saveHint')}</span></div></div>
      <div className={styles.actionButtons}>
        {dirty && onDiscard && <Button type='text' disabled={busy} onClick={onDiscard}>{t('agentSettings.workbench.resetChanges')}</Button>}
        {dirty || !editor.preset.current_stable_revision ? <AgentEditorActionButton icon={<Save theme='outline' size={15} fill='currentColor' />} loading={busyAction === 'save'} disabled={busy || unavailableCount > 0 || !draft.display_name.trim() || modelBlocksSave || idmmPolicyBlocksSave} onClick={onSave}>{saveLabel ?? t('common.save')}</AgentEditorActionButton> :
          <AgentConversationAction hasStableRevision={Boolean(editor.preset.current_stable_revision)} dirty={dirty} busy={busy} onClick={() => onStartConversation(editor.preset)} />}
      </div>
    </AgentEditorActionBar>
  </main>;
};

export default AgentPresetEditor;
