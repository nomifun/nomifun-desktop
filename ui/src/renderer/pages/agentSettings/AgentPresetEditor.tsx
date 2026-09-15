import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetSummary,
  CapabilityPlacement,
  ChatRouteCandidate,
  ChatRouteRecord,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  AGENT_CHAT_MODEL_TASK,
  missingSkillCapabilities,
  toggleSkill,
} from '@/common/types/agentPlatform';
import {
  Alert,
  Button,
  Checkbox,
  Collapse,
  Input,
  Select,
  Tag,
} from '@arco-design/web-react';
import {
  LinkCloud,
  MessageOne,
  Save,
  User,
} from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import AgentRuntimeEngineSelector from './AgentRuntimeEngineSelector';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import AgentContributionOrder from './AgentContributionOrder';
import { unavailableCapabilityReferences } from './capabilityGroups';
import {
  TEMPLATE_I18N_PATH,
  capabilityPlacement,
  capabilityReferenceKey,
  chatRouteCandidateKey,
  selectChatRouteCandidate,
  updateDocument,
} from './model';
import styles from './AgentSettingsPage.module.css';

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
    <Button
      type='primary'
      icon={<MessageOne theme='outline' size='15' />}
      disabled={busy || !hasStableRevision || dirty}
      onClick={onClick}
    >
      {t('agentSettings.actions.startConversation')}
    </Button>
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
  onSave, onDiscard, onOpenModels, onOpenAuthor, onStartConversation,
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState('capabilities');
  const busy = busyAction !== null;
  const unavailableCount = unavailableCapabilityReferences(draft.document, catalog.capabilities).length;
  const chatRouteRecord = draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK];
  const selectedSkills = new Set(draft.document.skill_bindings.map((skill) => skill.id));
  const catalogByReference = useMemo(() => new Map(catalog.capabilities.map((item) => [capabilityReferenceKey(item.capability), item])), [catalog.capabilities]);
  const patchDocument = (transform: Parameters<typeof updateDocument>[1]) => onDraftChange(updateDocument(draft, transform));
  const applyChatRouteRecord = (record: ChatRouteRecord) => patchDocument((document) => ({
    ...document,
    model_route_refs: { ...document.model_route_refs, [AGENT_CHAT_MODEL_TASK]: record.primary.model_route_id },
    chat_route_records: { ...document.chat_route_records, [AGENT_CHAT_MODEL_TASK]: record },
  }));
  const placementLabel = (placement: CapabilityPlacement): string => t(
    placement === 'enabled' ? 'agentSettings.capabilities.enabled' : 'agentSettings.capabilities.notSelected'
  );
  const tabs = [
    { key: 'capabilities', label: t('agentSettings.workbench.capabilityTab') },
    { key: 'providers', label: t('agentSettings.providers.title') },
    { key: 'settings', label: t('agentSettings.workbench.settingsTab') },
    { key: 'extensions', label: t('agentSettings.workbench.skillsTab') },
  ];

  return <main className={styles.editorSurface}>
    <header className={styles.editorHeader}>
      <span className={styles.agentAvatar}><User theme='outline' size={25} /></span>
      <div className={styles.editorHeaderCopy}>
        <div className={styles.headerEyebrow}>{t('agentSettings.workbench.personalBadge')}</div>
        <h2>{draft.display_name || t('agentSettings.defaults.untitledName')}</h2>
        <p>{draft.description || t('agentSettings.workbench.selectedHint')}</p>
      </div>
      {sourceTemplate && <Tag size='small'>{t(`agentSettings.template.${TEMPLATE_I18N_PATH[sourceTemplate.template_key]}.name`)}</Tag>}
    </header>
    <nav className={styles.editorTabs} role='tablist' aria-label={t('agentSettings.title')}>
      {tabs.map((tab) => <button key={tab.key} type='button' role='tab' aria-selected={activeTab === tab.key} aria-controls={`agent-panel-${tab.key}`} id={`agent-tab-${tab.key}`} className={activeTab === tab.key ? styles.activeTab : ''} onClick={() => setActiveTab(tab.key)}>{tab.label}</button>)}
    </nav>
    <div className={`${styles.editorBody} ${activeTab === 'capabilities' ? styles.capabilityBody : ''}`}>
      {activeTab === 'capabilities' && <div className={styles.capabilityPanel} role='tabpanel' id='agent-panel-capabilities' aria-labelledby='agent-tab-capabilities'>
        <AgentCapabilityWorkspace document={draft.document} catalog={catalog.capabilities} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
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
            />
          </label>
          <label className={styles.field}>
            <span>{t('common.model')}</span>
            <AgentChatModelPicker
              record={chatRouteRecord}
              disabled={busy}
              onChange={applyChatRouteRecord}
            />
          </label>
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
          <div className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.runtimeEngine.label')}</span>
            <AgentRuntimeEngineSelector
              value={draft.document.runtime_engine}
              disabled={busy}
              onChange={(runtime_engine) => patchDocument((document) => ({ ...document, runtime_engine }))}
            />
          </div>
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
        {!chatRouteRecord && (
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
        {!chatRouteRecord && onOpenModels && <Button type='text' onClick={onOpenModels}>{t('agentSettings.workbench.manageModels')}</Button>}
      </div>}
      {activeTab === 'extensions' && <div role='tabpanel' id='agent-panel-extensions' aria-labelledby='agent-tab-extensions'>
        <AgentContributionOrder kind='middleware' document={draft.document} catalog={catalog.capabilities} disabled={busy} onOpenAuthor={onOpenAuthor} onChange={(document) => onDraftChange({ ...draft, document })} />
        <AgentContributionOrder document={draft.document} catalog={catalog.capabilities} disabled={busy} onChange={(document) => onDraftChange({ ...draft, document })} />
        <section className={styles.section} id='agent-settings-skills-mcp'>
        <Collapse defaultActiveKey={[]} className={styles.advancedCollapse}>
          <Collapse.Item name='skills-mcp' header={t('agentSettings.sections.skillsMcp')}>
            <p className={styles.collapseHint}>{t('agentSettings.sections.skillsMcpHint')}</p>
            <div className={styles.dualGrid}>
              <div className={styles.selectionColumn}>
                <div className={styles.selectionHeader}>
                  <span>{t('agentSettings.sections.skills')}</span>
                  <span>{draft.document.skill_bindings.length}</span>
                </div>
                <div className={styles.selectionList}>
                  {catalog.skills.map((skill) => {
                    const missing = missingSkillCapabilities(skill, draft.document);
                    return (
                      <label key={skill.skill.id} className={styles.skillRow}>
                        <Checkbox
                          checked={selectedSkills.has(skill.skill.id)}
                          disabled={busy}
                          onChange={() =>
                            patchDocument((document) => toggleSkill(document, skill.skill))
                          }
                        />
                        <div>
                          <strong>{skill.display_name}</strong>
                          <span>{skill.description}</span>
                          {missing.length > 0 && (
                            <small>
                              {t('agentSettings.skills.missingCapabilities', {
                                capabilities: missing.join(', '),
                              })}
                            </small>
                          )}
                        </div>
                      </label>
                    );
                  })}
                  {catalog.skills.length === 0 && (
                    <div className={styles.inlineEmpty}>{t('agentSettings.skills.empty')}</div>
                  )}
                </div>
              </div>

              <div className={styles.selectionColumn}>
                <div className={styles.selectionHeader}>
                  <span>{t('agentSettings.sections.mcp')}</span>
                  <span>{catalog.mcp_tools.length}</span>
                </div>
                <div className={styles.selectionList}>
                  {catalog.mcp_tools.map((mapping) => {
                    const capability = catalogByReference.get(
                      capabilityReferenceKey(mapping.capability)
                    );
                    const placement = capabilityPlacement(
                      draft.document,
                      mapping.capability
                    );
                    return (
                      <div
                        key={`${mapping.server_id}:${mapping.canonical_tool_key}`}
                        className={styles.mcpRow}
                      >
                        <LinkCloud theme='outline' size='15' />
                        <div>
                          <strong>{mapping.canonical_tool_key}</strong>
                          <span>
                            {capability?.display_name ?? mapping.capability.id}
                          </span>
                        </div>
                        <Tag
                          size='small'
                          color={placement === 'none' ? 'gray' : 'blue'}
                        >
                          {placementLabel(placement)}
                        </Tag>
                      </div>
                    );
                  })}
                  {catalog.mcp_tools.length === 0 && (
                    <div className={styles.inlineEmpty}>{t('agentSettings.mcp.empty')}</div>
                  )}
                </div>
              </div>
            </div>
          </Collapse.Item>
        </Collapse>
      </section></div>}
    </div>
    <footer className={styles.actionBar}>
      <div className={styles.saveStatus}><span className={dirty || unavailableCount ? styles.statusWarningDot : styles.statusReadyDot} /><div><strong>{t(dirty ? 'agentSettings.workbench.pendingChanges' : 'agentSettings.workbench.savedHint')}</strong><span>{t(unavailableCount ? 'agentSettings.workbench.disabledSave' : !chatRouteRecord ? 'agentSettings.workbench.modelNeeded' : 'agentSettings.workbench.saveHint')}</span></div></div>
      <div className={styles.actionButtons}>
        {dirty && onDiscard && <Button type='text' disabled={busy} onClick={onDiscard}>{t('agentSettings.workbench.resetChanges')}</Button>}
        {dirty || !editor.preset.current_stable_revision ? <Button type='primary' icon={<Save theme='outline' size={15} />} loading={busyAction === 'save'} disabled={busy || unavailableCount > 0 || !draft.display_name.trim() || !chatRouteRecord} onClick={onSave}>{t('common.save')}</Button> :
          <AgentConversationAction hasStableRevision={Boolean(editor.preset.current_stable_revision)} dirty={dirty} busy={busy} onClick={() => onStartConversation(editor.preset)} />}
      </div>
    </footer>
  </main>;
};

export default AgentPresetEditor;
