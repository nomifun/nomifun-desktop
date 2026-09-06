import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  AgentPresetSummary,
  CapabilityPlacement,
  ChatRouteCandidate,
  ChatRouteRecord,
  InstallationTokenStateResponse,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
  RunAgentPresetTestResult,
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
  Info,
  LinkCloud,
  MessageOne,
  PlayOne,
  PreviewOpen,
  Save,
  Search,
} from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import AgentCapabilityList from './AgentCapabilityList';
import {
  TEMPLATE_I18N_PATH,
  capabilityMatchesSearch,
  capabilityPlacement,
  capabilityReferenceKey,
  chatRouteCandidateKey,
  editorCapabilityReferences,
  placeCapability,
  previewDiagnosticMessage,
  selectChatRouteCandidate,
  selectedRequiredResourceKinds,
  sortCapabilitiesByPlacement,
  updateDocument,
} from './model';
import PreviewInspector from './PreviewInspector';
import styles from './AgentSettingsPage.module.css';

type AgentPresetEditorProps = {
  editor: AgentPresetEditorResponse;
  draft: AgentPresetDraft;
  catalog: AgentCatalogResponse;
  preview: ResolveAgentPresetPreviewResponse | null;
  testResult: RunAgentPresetTestResult | null;
  tokenState: InstallationTokenStateResponse | null;
  sourceTemplate?: OfficialPresetTemplate;
  busyAction:
    | 'preview'
    | 'save'
    | 'test'
    | 'fork'
    | 'create'
    | 'open'
    | 'delete'
    | null;
  dirty: boolean;
  onDraftChange: (draft: AgentPresetDraft) => void;
  onPreview: () => void;
  onSave: () => void;
  onTest: (input: string) => void;
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
  editor,
  draft,
  catalog,
  preview,
  testResult,
  tokenState,
  sourceTemplate,
  busyAction,
  dirty,
  onDraftChange,
  onPreview,
  onSave,
  onTest,
  onStartConversation,
}) => {
  const { t, i18n } = useTranslation();
  const [capabilitySearch, setCapabilitySearch] = useState('');
  const [testInput, setTestInput] = useState('');
  const catalogByReference = useMemo(
    () =>
      new Map(
        catalog.capabilities.map((item) => [
          capabilityReferenceKey(item.capability),
          item,
        ])
      ),
    [catalog.capabilities]
  );
  const filteredCapabilities = useMemo(() => {
    return catalog.capabilities.filter((item) =>
      capabilityMatchesSearch(item, capabilitySearch, i18n.language)
    );
  }, [capabilitySearch, catalog.capabilities, i18n.language]);
  const visibleCapabilities = useMemo(
    () => sortCapabilitiesByPlacement(draft.document, filteredCapabilities),
    [draft.document, filteredCapabilities]
  );
  const visibleCapabilityReferences = useMemo(
    () =>
      editorCapabilityReferences(
        draft.document,
        visibleCapabilities,
        catalog.capabilities
      ),
    [catalog.capabilities, draft.document, visibleCapabilities]
  );
  const selectedSkills = new Set(draft.document.skill_bindings.map((skill) => skill.id));
  const chatRouteRecord = draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK];
  const previewBlocked = preview?.status === 'blocked';
  const initialCount = draft.document.initial_capabilities.length;
  const onDemandCount = draft.document.on_demand_capabilities.length;
  const requiredResourceKinds = useMemo(
    () => selectedRequiredResourceKinds(draft.document, catalog.capabilities),
    [catalog.capabilities, draft.document]
  );
  const busy = busyAction !== null;

  const patchDocument = (transform: Parameters<typeof updateDocument>[1]) =>
    onDraftChange(updateDocument(draft, transform));
  const setPlacement = (
    capability: (typeof catalog.capabilities)[number]['capability'],
    placement: CapabilityPlacement
  ) => patchDocument((document) => placeCapability(document, capability, placement));
  const applyChatRouteRecord = (record: ChatRouteRecord) => {
    patchDocument((document) => ({
      ...document,
      model_route_refs: {
        ...document.model_route_refs,
        [AGENT_CHAT_MODEL_TASK]: record.primary.model_route_id,
      },
      chat_route_records: {
        ...document.chat_route_records,
        [AGENT_CHAT_MODEL_TASK]: record,
      },
    }));
  };
  const placementLabel = (placement: CapabilityPlacement): string => {
    switch (placement) {
      case 'initial':
        return t('agentSettings.capabilities.initialShort');
      case 'on_demand':
        return t('agentSettings.capabilities.onDemandShort');
      case 'none':
        return t('agentSettings.capabilities.notSelected');
    }
  };

  return (
    <main className={styles.editorSurface}>
      <header className={styles.editorHeader}>
        <div className={styles.editorHeaderCopy}>
          <h2>{draft.display_name}</h2>
          {draft.description && <p>{draft.description}</p>}
        </div>
        <div className={styles.tagRow}>
          {sourceTemplate && (
            <Tag size='small' color='gray'>
              {t(`agentSettings.template.${TEMPLATE_I18N_PATH[sourceTemplate.template_key]}.name`)}
            </Tag>
          )}
        </div>
      </header>

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
                'This host did not provide an available Chat model route. Saving or testing remains blocked until one is available.',
            })}
          />
        )}
      </section>

      <section className={styles.section} id='agent-settings-capabilities'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.capabilities')}</h3>
            <p>{t('agentSettings.sections.capabilitiesHint')}</p>
          </div>
          <div className={styles.searchField}>
            <Search theme='outline' size='14' />
            <input
              value={capabilitySearch}
              aria-label={t('agentSettings.capabilities.search')}
              placeholder={t('agentSettings.capabilities.search')}
              disabled={busy}
              onChange={(event: React.ChangeEvent<HTMLInputElement>) =>
                setCapabilitySearch(event.target.value)
              }
            />
          </div>
        </div>

        <div className={styles.capabilitySelectionSummary}>
          <span>
            {t('agentSettings.capabilities.initial')}: <strong>{initialCount}</strong>
          </span>
          <span>
            {t('agentSettings.capabilities.onDemand')}: <strong>{onDemandCount}</strong>
          </span>
        </div>

        <AgentCapabilityList
          references={visibleCapabilityReferences}
          catalog={catalog.capabilities}
          emptyLabel={t('agentSettings.capabilities.emptyCatalog')}
          disabled={busy}
          placementFor={(capability) =>
            capabilityPlacement(draft.document, capability)
          }
          onPlacementChange={setPlacement}
        />

        <div className={styles.requirementPolicy}>
          <strong>{t('agentSettings.resources.bindingPolicyTitle')}</strong>
          <span>{t('agentSettings.resources.bindingPolicyBody')}</span>
        </div>
      </section>

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
      </section>

      {preview?.status === 'blocked' && (
        <div className={styles.previewNotice}>
          <Alert
            type='error'
            showIcon
            content={
              preview.diagnostics[0]
                ? previewDiagnosticMessage(preview.diagnostics[0])
                : t('agentSettings.preview.blocked')
            }
          />
        </div>
      )}

      <section className={styles.section} id='agent-settings-test'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.actions.test')}</h3>
            <p>{t('agentSettings.sections.testHint')}</p>
          </div>
        </div>
        <Alert
          type='warning'
          showIcon
          icon={<Info theme='outline' size='16' />}
          content={t('agentSettings.test.realEffectWarning')}
        />
        <div className={styles.testComposer}>
          <Input.TextArea
            value={testInput}
            autoSize={{ minRows: 2, maxRows: 6 }}
            placeholder={t('agentSettings.test.inputPlaceholder')}
            disabled={busy}
            onChange={setTestInput}
          />
          <Button
            type='primary'
            icon={<PlayOne theme='outline' size='15' />}
            loading={busyAction === 'test'}
            disabled={busy || !testInput.trim() || previewBlocked}
            onClick={() => onTest(testInput.trim())}
          >
            {t('agentSettings.actions.test')}
          </Button>
        </div>
        {testResult && (
          <div className={styles.testResultSummary}>
            <span>{t('common.success')}</span>
            <Button
              type='secondary'
              size='small'
              onClick={() => {
                window.location.hash = `/agent-sessions/${testResult.session.agent_session_id}`;
              }}
            >
              {t('agentSettings.session.open')}
            </Button>
          </div>
        )}
      </section>

      <section className={styles.section} id='agent-settings-preview'>
        <Collapse defaultActiveKey={[]} className={styles.technicalCollapse}>
          <Collapse.Item name='technical-details' header={t('common.technical_details')}>
            <div className={styles.technicalStack}>
              <div className={styles.technicalHeader}>
                <div>
                  <strong>{t('agentSettings.sections.previewInspector')}</strong>
                  <span>{t('agentSettings.sections.previewInspectorHint')}</span>
                </div>
                  <Button
                    size='small'
                    icon={<PreviewOpen theme='outline' size='15' />}
                    loading={busyAction === 'preview'}
                    disabled={busy}
                    onClick={onPreview}
                >
                  {t('agentSettings.actions.preview')}
                </Button>
              </div>
              <PreviewInspector preview={preview} tokenState={tokenState} />

              <div className={styles.technicalGroup}>
                <div className={styles.technicalGroupHeader}>
                  <strong>{t('agentSettings.fields.chatModelRoute')}</strong>
                  <span>{t('agentSettings.sections.basicHint')}</span>
                </div>
                <Tag size='small' color={chatRouteRecord ? 'green' : 'orange'}>
                  {chatRouteRecord
                    ? t('agentSettings.common.available')
                    : t('agentSettings.common.unavailable')}
                </Tag>
              </div>

              <div className={styles.inspectorRows}>
                <div>
                  <span>{t('agentSettings.status.currentRevision', { revision: '' })}</span>
                  <strong>{editor.revision?.reference.revision ?? 0}</strong>
                </div>
                <div>
                  <span>
                    {t('agentSettings.library.bindingCount', {
                      count: editor.preset.bound_target_count,
                    })}
                  </span>
                  <strong>{editor.preset.bound_target_count}</strong>
                </div>
                <div>
                  <span>{t('agentSettings.resources.requiredKinds')}</span>
                  <strong>{requiredResourceKinds.length}</strong>
                </div>
              </div>

              {testResult && (
                <div className={styles.technicalGroup}>
                  <div className={styles.technicalGroupHeader}>
                    <strong>{t('agentSettings.sections.test')}</strong>
                    <span>{t('common.success')}</span>
                  </div>
                  <div className={styles.inspectorRows}>
                    <div>
                      <span>{t('agentSettings.test.session')}</span>
                      <strong>{t('common.success')}</strong>
                    </div>
                    <div>
                      <span>{t('agentSettings.test.turn')}</span>
                      <strong>{t('common.success')}</strong>
                    </div>
                    <div>
                      <span>{t('agentSettings.test.revision')}</span>
                      <strong>
                        {testResult.savedRevision
                          ? testResult.savedRevision.revision.reference.revision
                          : testResult.preview.candidate_revision_ref.revision}
                      </strong>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </Collapse.Item>
        </Collapse>
      </section>

      <footer className={styles.actionBar}>
        <AgentConversationAction
          hasStableRevision={Boolean(editor.preset.current_stable_revision)}
          dirty={dirty}
          busy={busy}
          onClick={() => onStartConversation(editor.preset)}
        />
        <div className={styles.actionButtons}>
          <Button
            icon={<PlayOne theme='outline' size='15' />}
            loading={busyAction === 'test'}
            disabled={busy || !testInput.trim() || previewBlocked}
            onClick={() => onTest(testInput.trim())}
          >
            {t('agentSettings.actions.test')}
          </Button>
          <Button
            type='primary'
            icon={<Save theme='outline' size='15' />}
            loading={busyAction === 'save'}
            disabled={busy || previewBlocked}
            onClick={onSave}
          >
            {t('common.save')}
          </Button>
        </div>
      </footer>
    </main>
  );
};

export default AgentPresetEditor;
