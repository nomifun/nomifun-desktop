import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  CapabilityCatalogItem,
  CapabilityPlacement,
  ChatRouteRecord,
  ChatRouteCandidate,
  InstallationTokenStateResponse,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
} from '@/common/types/agentPlatform';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import {
  AGENT_CHAT_MODEL_TASK,
  CHAT_ROUTE_RECORD_SCHEMA,
  capabilityPlacement,
  missingSkillCapabilities,
  placeCapability,
  toggleSkill,
} from '@/common/types/agentPlatform';
import type { RunAgentPresetTestResult } from '@/common/types/agentPlatform';
import {
  Alert,
  Button,
  Checkbox,
  Collapse,
  Input,
  InputNumber,
  Select,
  Tag,
  Tooltip,
} from '@arco-design/web-react';
import { CloseSmall, Info, LinkCloud, PlayOne, PreviewOpen, Save, Search } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { WorkspaceFolderSelect } from '@/renderer/components/workspace';
import {
  DEFAULT_WORKSPACE_RESOURCE_ID,
  WORKSPACE_ROOT_PARAMETER,
  bindWorkspaceResource,
  bindKnowledgeBaseResource,
  chatRouteCandidateKey,
  defaultResourceBinding,
  removeResourceBinding,
  resourceKindsForDraft,
  selectChatRouteCandidate,
  TEMPLATE_I18N_PATH,
  updateDocument,
  updateResourceBinding,
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
  hostWorkDir: string | null;
  connectors: IMcpServer[];
  knowledgeBases: IKnowledgeBase[];
  knowledgeBasesLoading: boolean;
  sourceTemplate?: OfficialPresetTemplate;
  busyAction: 'preview' | 'save' | 'test' | 'fork' | 'create' | null;
  onDraftChange: (draft: AgentPresetDraft) => void;
  onPreview: () => void;
  onSave: () => void;
  onTest: (input: string) => void;
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
    return providerName ? `${providerName} · ${modelName}` : modelName;
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

const SelectedCapabilityList: React.FC<{
  title: string;
  items: AgentPresetDraft['document']['initial_capabilities'];
  catalogById: Map<string, CapabilityCatalogItem>;
  onRemove: (item: CapabilityCatalogItem['capability']) => void;
}> = ({ title, items, catalogById, onRemove }) => {
  const { t } = useTranslation();
  return (
    <div className={styles.selectionColumn}>
      <div className={styles.selectionHeader}>
        <span>{title}</span>
        <span>{items.length}</span>
      </div>
      {items.length === 0 ? (
        <div className={styles.inlineEmpty}>{t('agentSettings.capabilities.emptySelection')}</div>
      ) : (
        <div className={styles.selectionList}>
          {items.map((selection) => {
            const catalogItem = catalogById.get(selection.capability.id);
            return (
              <div key={selection.capability.id} className={styles.selectionRow}>
                <div>
                  <strong>{catalogItem?.display_name ?? selection.capability.id}</strong>
                  <span>
                    {selection.capability.id}@{selection.capability.version}
                  </span>
                </div>
                <Tooltip content={t('agentSettings.actions.remove')}>
                  <Button
                    type='text'
                    size='mini'
                    icon={<CloseSmall theme='outline' size='14' />}
                    onClick={() => onRemove(selection.capability)}
                  />
                </Tooltip>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

const numberValue = (record: Record<string, unknown>, key: string, fallback: number): number =>
  typeof record[key] === 'number' ? (record[key] as number) : fallback;

const AgentPresetEditor: React.FC<AgentPresetEditorProps> = ({
  editor,
  draft,
  catalog,
  preview,
  testResult,
  tokenState,
  hostWorkDir,
  connectors,
  knowledgeBases,
  knowledgeBasesLoading,
  sourceTemplate,
  busyAction,
  onDraftChange,
  onPreview,
  onSave,
  onTest,
}) => {
  const { t } = useTranslation();
  const [capabilitySearch, setCapabilitySearch] = useState('');
  const [testInput, setTestInput] = useState('');

  const catalogById = useMemo(
    () => new Map(catalog.capabilities.map((item) => [item.capability.id, item])),
    [catalog.capabilities]
  );
  const filteredCapabilities = useMemo(() => {
    const query = capabilitySearch.trim().toLowerCase();
    if (!query) return catalog.capabilities;
    return catalog.capabilities.filter((item) =>
      [item.display_name, item.description, item.capability.id, item.source_package.id]
        .join(' ')
        .toLowerCase()
        .includes(query)
    );
  }, [capabilitySearch, catalog.capabilities]);
  const requiredKinds = useMemo(
    () => resourceKindsForDraft(draft, catalog.capabilities),
    [catalog.capabilities, draft]
  );
  const templateDefaults = useMemo(
    () =>
      new Map(
        (sourceTemplate?.seed.typed_resource_defaults ?? []).map((resource) => [
          resource.resource_kind,
          resource,
        ])
      ),
    [sourceTemplate]
  );

  const patchDocument = (transform: Parameters<typeof updateDocument>[1]) =>
    onDraftChange(updateDocument(draft, transform));
  const place = (capability: CapabilityCatalogItem['capability'], placement: CapabilityPlacement) =>
    patchDocument((document) => placeCapability(document, capability, placement));

  const selectedSkills = new Set(draft.document.skill_bindings.map((skill) => skill.id));
  const chatRouteRecord = draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK];
  const [chatRouteRecordText, setChatRouteRecordText] = useState(() =>
    chatRouteRecord ? JSON.stringify(chatRouteRecord, null, 2) : ''
  );
  useEffect(() => {
    setChatRouteRecordText(chatRouteRecord ? JSON.stringify(chatRouteRecord, null, 2) : '');
  }, [chatRouteRecord]);
  const previewBlocked = preview?.status === 'blocked';
  const selectedCapabilityIds = useMemo(
    () =>
      new Set([
        ...draft.document.initial_capabilities.map((item) => item.capability.id),
        ...draft.document.on_demand_capabilities.map((item) => item.capability.id),
      ]),
    [draft.document.initial_capabilities, draft.document.on_demand_capabilities]
  );

  const resourceLabelFor = (resourceKind: string): string => {
    switch (resourceKind) {
      case 'workspace':
        return t('terminal.create.workspace');
      case 'knowledge_base':
        return t('agentSettings.resources.knowledgeBase');
      case 'mcp_server':
        return t('agentSettings.sections.mcp');
      case 'process_session':
        return t('agentSettings.sections.test');
      case 'companion':
        return t('agentSettings.template.companion.default.name');
      case 'companion_memory':
        return t('agentSettings.template.companion.default.name');
      case 'channel':
        return t('agentSettings.template.customerService.default.name');
      case 'customer':
        return t('agentSettings.template.customerService.default.name');
      case 'robot':
        return t('agentSettings.template.robot.default.name');
      case 'canvas':
      case 'asset_library':
      case 'generation_provider':
      case 'miniapp':
        return t('agentSettings.template.creativeStudio.default.name');
      case 'project_memory':
        return t('agentSettings.sections.resources');
      default:
        return t('agentSettings.sections.resources');
    }
  };

  const applyChatRouteRecord = (record: ChatRouteRecord) => {
    setChatRouteRecordText(JSON.stringify(record, null, 2));
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

  const applyChatRouteRecordText = (text: string) => {
    setChatRouteRecordText(text);
    if (!text.trim()) {
      patchDocument((document) => {
        const modelRouteRefs = { ...document.model_route_refs };
        const chatRouteRecords = { ...document.chat_route_records };
        delete modelRouteRefs[AGENT_CHAT_MODEL_TASK];
        delete chatRouteRecords[AGENT_CHAT_MODEL_TASK];
        return {
          ...document,
          model_route_refs: modelRouteRefs,
          chat_route_records: chatRouteRecords,
        };
      });
      return;
    }
    try {
      const parsed = JSON.parse(text) as ChatRouteRecord;
      if (
        parsed.schema !== CHAT_ROUTE_RECORD_SCHEMA ||
        parsed.task !== AGENT_CHAT_MODEL_TASK ||
        !parsed.primary?.model_route_id
      ) {
        return;
      }
      applyChatRouteRecord(parsed);
    } catch {
      // Keep the draft unchanged until the JSON is complete and valid.
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
          </div>
        </div>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>{t('agentSettings.fields.name')}</span>
            <Input
              value={draft.display_name}
              maxLength={80}
              onChange={(displayName: string) =>
                onDraftChange({ ...draft, display_name: displayName })
              }
            />
          </label>
          <label className={styles.field}>
            <span>{t('common.model')}</span>
            <AgentChatModelPicker
              record={chatRouteRecord}
              disabled={busyAction !== null}
              onChange={applyChatRouteRecord}
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.description')}</span>
            <Input
              value={draft.description ?? ''}
              maxLength={240}
              onChange={(description: string) =>
                onDraftChange({
                  ...draft,
                  description: description || undefined,
                })
              }
            />
          </label>
        </div>
      </section>

      <section className={styles.section} id='agent-settings-capabilities'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.capabilities')}</h3>
          </div>
          <div className={styles.searchField}>
            <Search theme='outline' size='14' />
            <input
              value={capabilitySearch}
              aria-label={t('agentSettings.capabilities.search')}
              placeholder={t('agentSettings.capabilities.search')}
              onChange={(event: React.ChangeEvent<HTMLInputElement>) =>
                setCapabilitySearch(event.target.value)
              }
            />
          </div>
        </div>

        <div className={styles.enabledSummary}>
          <span>{t('agentSettings.sections.capabilities')}</span>
          <strong>{selectedCapabilityIds.size}</strong>
        </div>

        <div className={styles.catalogList}>
          {filteredCapabilities.map((item) => {
            const placement = capabilityPlacement(draft.document, item.capability.id);
            const unavailable = item.materialization_state === 'unavailable';
            return (
              <div key={item.capability.id} className={styles.catalogRow}>
                <div className={styles.catalogCopy}>
                  <div className={styles.catalogTitle}>
                    <strong>{item.display_name}</strong>
                    {unavailable && (
                      <Tag size='small' color='orange'>
                        {t('agentSettings.common.unavailable')}
                      </Tag>
                    )}
                  </div>
                  <p>{item.description}</p>
                </div>
                <Checkbox
                  checked={placement !== 'none'}
                  disabled={unavailable || busyAction !== null}
                  aria-label={item.display_name}
                  onChange={(checked: boolean) =>
                    place(
                      item.capability,
                      checked ? (placement === 'none' ? 'initial' : placement) : 'none'
                    )
                  }
                />
              </div>
            );
          })}
          {filteredCapabilities.length === 0 && (
            <div className={styles.inlineEmpty}>{t('agentSettings.capabilities.emptyCatalog')}</div>
          )}
        </div>
        <Collapse defaultActiveKey={[]} className={styles.technicalCollapse}>
          <Collapse.Item name='capability-details' header={t('common.technical_details')}>
            <div className={styles.dualGrid}>
              <SelectedCapabilityList
                title={t('agentSettings.capabilities.initial')}
                items={draft.document.initial_capabilities}
                catalogById={catalogById}
                onRemove={(capability) => place(capability, 'none')}
              />
              <SelectedCapabilityList
                title={t('agentSettings.capabilities.onDemand')}
                items={draft.document.on_demand_capabilities}
                catalogById={catalogById}
                onRemove={(capability) => place(capability, 'none')}
              />
            </div>
          </Collapse.Item>
        </Collapse>
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
                          onChange={() =>
                            patchDocument((document) => toggleSkill(document, skill.skill))
                          }
                        />
                        <div>
                          <strong>{skill.display_name}</strong>
                          <span>{skill.description}</span>
                          {missing.length > 0 && (
                            <small>{t('agentSettings.common.unavailable')}</small>
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
                    const capability = catalogById.get(mapping.capability.id);
                    const placement = capabilityPlacement(draft.document, mapping.capability.id);
                    return (
                      <div
                        key={`${mapping.server_id}:${mapping.canonical_tool_key}`}
                        className={styles.mcpRow}
                      >
                        <LinkCloud theme='outline' size='15' />
                        <div>
                          <strong>{mapping.canonical_tool_key}</strong>
                          <span>{t('agentSettings.sections.mcp')}</span>
                        </div>
                        <Checkbox
                          checked={placement !== 'none'}
                          disabled={!capability || busyAction !== null}
                          aria-label={mapping.canonical_tool_key}
                          onChange={(checked: boolean) =>
                            place(
                              mapping.capability,
                              checked ? (placement === 'none' ? 'initial' : placement) : 'none'
                            )
                          }
                        />
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

      <section className={styles.section} id='agent-settings-resources'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.resources')}</h3>
          </div>
        </div>
        {requiredKinds.length === 0 ? (
          <div className={styles.inlineEmpty}>{t('agentSettings.resources.noneRequired')}</div>
        ) : (
          <div className={styles.resourceEditorList}>
            {requiredKinds.map((resourceKind) => {
              const existing = draft.document.resource_bindings.find(
                (binding) => binding.resource_kind === resourceKind
              );
              const resourceDefault = templateDefaults.get(resourceKind);
              const binding =
                existing ??
                defaultResourceBinding(
                  resourceKind,
                  editor.preset.owner_user_id ?? '',
                  resourceDefault?.operations ?? ['read'],
                  resourceKind === 'workspace' && hostWorkDir
                    ? {
                        resourceId: DEFAULT_WORKSPACE_RESOURCE_ID,
                        typedParameters: {
                          [WORKSPACE_ROOT_PARAMETER]: hostWorkDir,
                        },
                      }
                    : undefined
                );
              return (
                <div key={resourceKind} className={styles.resourceEditorRow}>
                  <div className={styles.resourceEditorHeader}>
                    <div>
                      <strong>{resourceLabelFor(resourceKind)}</strong>
                      <span>
                        {resourceDefault?.required
                          ? t('agentSettings.resources.required')
                          : t('agentSettings.resources.optional')}
                      </span>
                    </div>
                    {existing && (
                      <Tooltip content={t('agentSettings.actions.remove')}>
                        <Button
                          type='text'
                          size='mini'
                          icon={<CloseSmall theme='outline' size='14' />}
                          onClick={() =>
                            onDraftChange(removeResourceBinding(draft, existing.binding_id))
                          }
                        />
                      </Tooltip>
                    )}
                  </div>
                  <div className={styles.resourcePicker}>
                    {resourceKind === 'workspace' && (
                      <WorkspaceFolderSelect
                        value={
                          binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER] ?? hostWorkDir ?? ''
                        }
                        onChange={(workspaceRoot: string) =>
                          onDraftChange(
                            updateResourceBinding(
                              draft,
                              bindWorkspaceResource(binding, workspaceRoot)
                            )
                          )
                        }
                        onClear={() =>
                          onDraftChange(
                            updateResourceBinding(
                              draft,
                              bindWorkspaceResource(binding, hostWorkDir ?? '')
                            )
                          )
                        }
                        placeholder={t('terminal.create.workspacePlaceholder')}
                        recentLabel={t('terminal.create.recent')}
                        chooseDifferentLabel={t('terminal.create.chooseFolder')}
                      />
                    )}
                    {resourceKind === 'knowledge_base' && (
                      <Select
                        value={binding.resource_id || undefined}
                        loading={knowledgeBasesLoading}
                        showSearch
                        placeholder={t('agentSettings.resources.knowledgeBasePlaceholder')}
                        options={knowledgeBases.map((knowledgeBase) => ({
                          label: knowledgeBase.name,
                          value: knowledgeBase.knowledge_base_id,
                          disabled: !knowledgeBase.root_exists,
                        }))}
                        onChange={(knowledgeBaseId: string) => {
                          const knowledgeBase = knowledgeBases.find(
                            (candidate) =>
                              String(candidate.knowledge_base_id) === String(knowledgeBaseId)
                          );
                          if (!knowledgeBase) return;
                          onDraftChange(
                            updateResourceBinding(
                              draft,
                              bindKnowledgeBaseResource(binding, knowledgeBase)
                            )
                          );
                        }}
                      />
                    )}
                    {resourceKind === 'mcp_server' && (
                      <Select
                        value={binding.resource_id || undefined}
                        showSearch
                        disabled={connectors.length === 0}
                        placeholder={t('common.select')}
                        options={connectors.map((connector) => ({
                          label: connector.name,
                          value: String(connector.mcp_server_id),
                        }))}
                        onChange={(connectorId: string) =>
                          onDraftChange(
                            updateResourceBinding(draft, {
                              ...binding,
                              resource_id: connectorId,
                            })
                          )
                        }
                      />
                    )}
                    {!['workspace', 'knowledge_base', 'mcp_server'].includes(resourceKind) && (
                      <div className={styles.managedResource}>
                        <Tag size='small' color={binding.resource_id ? 'green' : 'gray'}>
                          {binding.resource_id ? t('common.added') : t('agentSettings.common.none')}
                        </Tag>
                        <span>
                          {binding.resource_id
                            ? t('agentSettings.resources.required')
                            : t('agentSettings.resources.optional')}
                        </span>
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className={styles.section} id='agent-settings-advanced'>
        <Collapse defaultActiveKey={[]} className={styles.advancedCollapse}>
          <Collapse.Item name='advanced' header={t('agentSettings.sections.advanced')}>
            <div className={styles.formGrid}>
              <label className={`${styles.field} ${styles.fieldWide}`}>
                <span>{t('agentSettings.fields.persona')}</span>
                <Input.TextArea
                  value={draft.document.persona}
                  autoSize={{ minRows: 2, maxRows: 5 }}
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
                  onChange={(instructions: string) =>
                    patchDocument((document) => ({ ...document, instructions }))
                  }
                />
              </label>
              <label className={styles.field}>
                <span>{t('agentSettings.advanced.systemTokens')}</span>
                <InputNumber
                  min={0}
                  value={numberValue(draft.document.context_policy, 'max_system_tokens', 12000)}
                  onChange={(value: number | undefined) =>
                    patchDocument((document) => ({
                      ...document,
                      context_policy: {
                        ...document.context_policy,
                        max_system_tokens: value ?? 0,
                      },
                    }))
                  }
                />
              </label>
              <label className={styles.field}>
                <span>{t('agentSettings.advanced.dynamicTokens')}</span>
                <InputNumber
                  min={0}
                  value={numberValue(
                    draft.document.context_policy,
                    'max_dynamic_context_tokens',
                    16000
                  )}
                  onChange={(value: number | undefined) =>
                    patchDocument((document) => ({
                      ...document,
                      context_policy: {
                        ...document.context_policy,
                        max_dynamic_context_tokens: value ?? 0,
                      },
                    }))
                  }
                />
              </label>
              <label className={styles.field}>
                <span>{t('agentSettings.advanced.activeCapabilities')}</span>
                <InputNumber
                  min={0}
                  value={numberValue(
                    draft.document.execution_constraints,
                    'max_active_capabilities',
                    64
                  )}
                  onChange={(value: number | undefined) =>
                    patchDocument((document) => ({
                      ...document,
                      execution_constraints: {
                        ...document.execution_constraints,
                        max_active_capabilities: value ?? 0,
                      },
                    }))
                  }
                />
              </label>
              <label className={styles.field}>
                <span>{t('agentSettings.advanced.toolCalls')}</span>
                <InputNumber
                  min={0}
                  value={numberValue(draft.document.runtime_budget, 'max_tool_calls_per_turn', 64)}
                  onChange={(value: number | undefined) =>
                    patchDocument((document) => ({
                      ...document,
                      runtime_budget: {
                        ...document.runtime_budget,
                        max_tool_calls_per_turn: value ?? 0,
                      },
                    }))
                  }
                />
              </label>
            </div>
          </Collapse.Item>
        </Collapse>
      </section>

      {preview?.status === 'blocked' && (
        <div className={styles.previewNotice}>
          <Alert
            type='error'
            showIcon
            content={preview.diagnostics[0]?.message ?? t('agentSettings.preview.blocked')}
          />
        </div>
      )}

      <section className={styles.section} id='agent-settings-test'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.actions.test')}</h3>
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
            onChange={setTestInput}
          />
          <Button
            type='primary'
            icon={<PlayOne theme='outline' size='15' />}
            loading={busyAction === 'test'}
            disabled={!testInput.trim() || previewBlocked}
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
                  onClick={onPreview}
                >
                  {t('agentSettings.actions.preview')}
                </Button>
              </div>
              <PreviewInspector preview={preview} tokenState={tokenState} />

              <div className={styles.technicalGroup}>
                <div className={styles.technicalGroupHeader}>
                  <strong>{t('agentSettings.fields.chatModelRouteRecord')}</strong>
                  <span>{t('agentSettings.fields.chatModelRoute')}</span>
                </div>
                <Input.TextArea
                  value={chatRouteRecordText}
                  placeholder={t('agentSettings.fields.chatModelRouteRecordPlaceholder')}
                  autoSize={{ minRows: 4, maxRows: 12 }}
                  onChange={applyChatRouteRecordText}
                />
              </div>

              <div className={styles.inspectorRows}>
                <div>
                  <span>{t('agentSettings.fields.chatModelRoute')}</span>
                  <code>
                    {draft.document.model_route_refs[AGENT_CHAT_MODEL_TASK] ??
                      t('agentSettings.common.unavailable')}
                  </code>
                </div>
                <div>
                  <span>
                    {t('agentSettings.status.currentRevision', {
                      revision: editor.revision?.reference.revision ?? 0,
                    })}
                  </span>
                  <code>
                    {editor.revision?.reference.revision_digest ??
                      t('agentSettings.common.unavailable')}
                  </code>
                </div>
                <div>
                  <span>
                    {t('agentSettings.library.bindingCount', {
                      count: editor.preset.bound_target_count,
                    })}
                  </span>
                  <code>{draft.preset_id}</code>
                </div>
              </div>

              {draft.document.resource_bindings.length > 0 && (
                <div className={styles.technicalGroup}>
                  <div className={styles.technicalGroupHeader}>
                    <strong>{t('agentSettings.sections.resources')}</strong>
                    <span>{t('common.technical_details')}</span>
                  </div>
                  <div className={styles.technicalBindingList}>
                    {draft.document.resource_bindings.map((binding) => (
                      <pre key={binding.binding_id} className={styles.technicalBinding}>
                        {JSON.stringify(binding, null, 2)}
                      </pre>
                    ))}
                  </div>
                </div>
              )}

              {testResult && (
                <div className={styles.technicalGroup}>
                  <div className={styles.technicalGroupHeader}>
                    <strong>{t('agentSettings.sections.test')}</strong>
                    <span>{t('agentSettings.test.session')}</span>
                  </div>
                  <div className={styles.inspectorRows}>
                    <div>
                      <span>{t('agentSettings.test.session')}</span>
                      <code>{testResult.session.agent_session_id}</code>
                    </div>
                    <div>
                      <span>{t('agentSettings.test.turn')}</span>
                      <code>{testResult.turn.status}</code>
                    </div>
                    <div>
                      <span>{t('agentSettings.test.revision')}</span>
                      <code>
                        {testResult.savedRevision
                          ? testResult.savedRevision.revision.reference.revision
                          : testResult.preview.candidate_revision_ref.revision}
                      </code>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </Collapse.Item>
        </Collapse>
      </section>

      <footer className={styles.actionBar}>
        <div className={styles.actionButtons}>
          <Button
            icon={<PlayOne theme='outline' size='15' />}
            loading={busyAction === 'test'}
            disabled={!testInput.trim() || previewBlocked}
            onClick={() => onTest(testInput.trim())}
          >
            {t('agentSettings.actions.test')}
          </Button>
          <Button
            type='primary'
            icon={<Save theme='outline' size='15' />}
            loading={busyAction === 'save'}
            disabled={previewBlocked}
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
