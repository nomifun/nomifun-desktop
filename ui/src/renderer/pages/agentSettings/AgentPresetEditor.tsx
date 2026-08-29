import type {
  AgentCatalogResponse,
  AgentPresetDraft,
  AgentPresetEditorResponse,
  CapabilityCatalogItem,
  CapabilityPlacement,
  InstallationTokenStateResponse,
  OfficialPresetTemplate,
  ResolveAgentPresetPreviewResponse,
} from '@/common/types/agentPlatform';
import {
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
  Radio,
  Tag,
  Tooltip,
} from '@arco-design/web-react';
import {
  BookOne,
  CloseSmall,
  Info,
  LinkCloud,
  PlayOne,
  PreviewOpen,
  Save,
  Search,
} from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  defaultResourceBinding,
  removeResourceBinding,
  resourceKindsForDraft,
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
  sourceTemplate?: OfficialPresetTemplate;
  dirty: boolean;
  busyAction: 'preview' | 'save' | 'test' | 'fork' | 'create' | null;
  onDraftChange: (draft: AgentPresetDraft) => void;
  onPreview: () => void;
  onSave: () => void;
  onTest: (input: string) => void;
};

const CapabilityPlacementControl: React.FC<{
  value: CapabilityPlacement;
  disabled: boolean;
  onChange: (placement: CapabilityPlacement) => void;
}> = ({ value, disabled, onChange }) => {
  const { t } = useTranslation();
  return (
    <Radio.Group
      type='button'
      size='mini'
      value={value}
      disabled={disabled}
      onChange={(next) => onChange(next as CapabilityPlacement)}
    >
      <Radio value='none'>{t('agentSettings.capabilities.notSelected')}</Radio>
      <Radio value='initial'>{t('agentSettings.capabilities.initialShort')}</Radio>
      <Radio value='on_demand'>{t('agentSettings.capabilities.onDemandShort')}</Radio>
    </Radio.Group>
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
  sourceTemplate,
  dirty,
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
  const place = (
    capability: CapabilityCatalogItem['capability'],
    placement: CapabilityPlacement
  ) => patchDocument((document) => placeCapability(document, capability, placement));

  const selectedSkills = new Set(draft.document.skill_bindings.map((skill) => skill.id));
  const chatRoute = draft.document.model_route_refs.chat ?? '';
  const previewBlocked = preview?.status === 'blocked';

  return (
    <main className={styles.editorSurface}>
      <header className={styles.editorHeader}>
        <div className={styles.editorHeaderCopy}>
          <div className={styles.eyebrow}>
            {dirty ? t('agentSettings.status.dirty') : t('agentSettings.status.saved')}
          </div>
          <h2>{draft.display_name}</h2>
          <p>
            {editor.revision
              ? t('agentSettings.status.currentRevision', {
                  revision: editor.revision.reference.revision,
                })
              : t('agentSettings.status.noRevision')}
          </p>
        </div>
        <div className={styles.tagRow}>
          {draft.source_template_key && (
            <Tag size='small' color='gray'>
              {draft.source_template_key}
            </Tag>
          )}
          {editor.preset.bound_target_count > 0 && (
            <Tag size='small' color='blue'>
              {t('agentSettings.status.boundTargets', {
                count: editor.preset.bound_target_count,
              })}
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
              onChange={(displayName) =>
                onDraftChange({ ...draft, display_name: displayName })
              }
            />
          </label>
          <label className={styles.field}>
            <span>{t('agentSettings.fields.chatModelRoute')}</span>
            <Input
              value={chatRoute}
              placeholder={t('agentSettings.fields.chatModelRoutePlaceholder')}
              onChange={(route) =>
                patchDocument((document) => ({
                  ...document,
                  model_route_refs: route.trim()
                    ? { ...document.model_route_refs, chat: route }
                    : Object.fromEntries(
                        Object.entries(document.model_route_refs).filter(
                          ([key]) => key !== 'chat'
                        )
                      ),
                }))
              }
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.description')}</span>
            <Input
              value={draft.description ?? ''}
              maxLength={240}
              onChange={(description) =>
                onDraftChange({ ...draft, description: description || undefined })
              }
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.persona')}</span>
            <Input.TextArea
              value={draft.document.persona}
              autoSize={{ minRows: 2, maxRows: 5 }}
              onChange={(persona) =>
                patchDocument((document) => ({ ...document, persona }))
              }
            />
          </label>
          <label className={`${styles.field} ${styles.fieldWide}`}>
            <span>{t('agentSettings.fields.instructions')}</span>
            <Input.TextArea
              value={draft.document.instructions}
              autoSize={{ minRows: 4, maxRows: 10 }}
              onChange={(instructions) =>
                patchDocument((document) => ({ ...document, instructions }))
              }
            />
          </label>
        </div>
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
              onChange={(event) => setCapabilitySearch(event.target.value)}
            />
          </div>
        </div>

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

        <div className={styles.catalogList}>
          {filteredCapabilities.map((item) => {
            const placement = capabilityPlacement(draft.document, item.capability.id);
            const unavailable = item.materialization_state === 'unavailable';
            return (
              <div key={item.capability.id} className={styles.catalogRow}>
                <div className={styles.catalogCopy}>
                  <div className={styles.catalogTitle}>
                    <strong>{item.display_name}</strong>
                    <code>{item.capability.id}</code>
                    {unavailable && (
                      <Tag size='small' color='red'>
                        {item.unavailable_code ?? 'CAPABILITY_NOT_MATERIALIZED'}
                      </Tag>
                    )}
                  </div>
                  <p>{item.description}</p>
                  <div className={styles.catalogMeta}>
                    <span>
                      {t('agentSettings.capabilities.source')}: {item.source_package.id}
                    </span>
                    <span>
                      {t('agentSettings.capabilities.tools', { count: item.action_count })}
                    </span>
                    <span>
                      {t('agentSettings.capabilities.contexts', {
                        count: item.context_contributor_count,
                      })}
                    </span>
                  </div>
                </div>
                <CapabilityPlacementControl
                  value={placement}
                  disabled={unavailable}
                  onChange={(next) => place(item.capability, next)}
                />
              </div>
            );
          })}
          {filteredCapabilities.length === 0 && (
            <div className={styles.inlineEmpty}>{t('agentSettings.capabilities.emptyCatalog')}</div>
          )}
        </div>
      </section>

      <section className={styles.section} id='agent-settings-skills-mcp'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.skillsMcp')}</h3>
            <p>{t('agentSettings.sections.skillsMcpHint')}</p>
          </div>
        </div>
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
                const capability = catalogById.get(mapping.capability.id);
                const placement = capabilityPlacement(
                  draft.document,
                  mapping.capability.id
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
                        {mapping.server_id} / {mapping.capability.id}
                      </span>
                    </div>
                    <CapabilityPlacementControl
                      value={placement}
                      disabled={!capability}
                      onChange={(next) => place(mapping.capability, next)}
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
      </section>

      <section className={styles.section} id='agent-settings-resources'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.resources')}</h3>
            <p>{t('agentSettings.sections.resourcesHint')}</p>
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
                  resourceDefault?.operations ?? ['read']
                );
              return (
                <div key={resourceKind} className={styles.resourceEditorRow}>
                  <div className={styles.resourceEditorHeader}>
                    <div>
                      <strong>{resourceKind}</strong>
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
                            onDraftChange(
                              removeResourceBinding(draft, existing.binding_id)
                            )
                          }
                        />
                      </Tooltip>
                    )}
                  </div>
                  <div className={styles.resourceFields}>
                    <label className={styles.field}>
                      <span>{t('agentSettings.resources.resourceId')}</span>
                      <Input
                        value={binding.resource_id}
                        onChange={(resourceId) =>
                          onDraftChange(
                            updateResourceBinding(draft, {
                              ...binding,
                              resource_id: resourceId,
                            })
                          )
                        }
                      />
                    </label>
                    <label className={styles.field}>
                      <span>{t('agentSettings.resources.operations')}</span>
                      <Input
                        value={binding.operations.join(', ')}
                        onChange={(operations) =>
                          onDraftChange(
                            updateResourceBinding(draft, {
                              ...binding,
                              operations: operations
                                .split(',')
                                .map((operation) => operation.trim())
                                .filter(Boolean),
                            })
                          )
                        }
                      />
                    </label>
                    <label className={styles.field}>
                      <span>{t('agentSettings.resources.connectionRef')}</span>
                      <Input
                        value={binding.connection_config_ref ?? ''}
                        onChange={(connectionRef) =>
                          onDraftChange(
                            updateResourceBinding(draft, {
                              ...binding,
                              connection_config_ref: connectionRef || undefined,
                            })
                          )
                        }
                      />
                    </label>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className={styles.section} id='agent-settings-advanced'>
        <Collapse className={styles.advancedCollapse}>
          <Collapse.Item
            name='advanced'
            header={t('agentSettings.sections.advanced')}
          >
            <div className={styles.formGrid}>
              <label className={styles.field}>
                <span>{t('agentSettings.advanced.systemTokens')}</span>
                <InputNumber
                  min={0}
                  value={numberValue(
                    draft.document.context_policy,
                    'max_system_tokens',
                    12000
                  )}
                  onChange={(value) =>
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
                  onChange={(value) =>
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
                  onChange={(value) =>
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
                  value={numberValue(
                    draft.document.runtime_budget,
                    'max_tool_calls_per_turn',
                    64
                  )}
                  onChange={(value) =>
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

      <section className={styles.section} id='agent-settings-preview'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.previewInspector')}</h3>
            <p>{t('agentSettings.sections.previewInspectorHint')}</p>
          </div>
        </div>
        <PreviewInspector preview={preview} tokenState={tokenState} />
      </section>

      <section className={styles.section} id='agent-settings-test'>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.test')}</h3>
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
          <div className={styles.testResult}>
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

      <footer className={styles.actionBar}>
        <div className={styles.actionStatus}>
          <BookOne theme='outline' size='16' />
          <span>
            {dirty
              ? t('agentSettings.status.unsavedChanges')
              : t('agentSettings.status.revisionImmutable')}
          </span>
        </div>
        <div className={styles.actionButtons}>
          <Button
            icon={<PreviewOpen theme='outline' size='15' />}
            loading={busyAction === 'preview'}
            onClick={onPreview}
          >
            {t('agentSettings.actions.preview')}
          </Button>
          <Button
            icon={<PlayOne theme='outline' size='15' />}
            loading={busyAction === 'test'}
            disabled={!testInput.trim()}
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
            {t('agentSettings.actions.saveRevision')}
          </Button>
        </div>
      </footer>
    </main>
  );
};

export default AgentPresetEditor;
