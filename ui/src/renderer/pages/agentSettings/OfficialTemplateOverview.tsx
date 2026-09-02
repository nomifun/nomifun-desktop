import type {
  ChatRouteRecord,
  OfficialPresetTemplate,
  TemplateResourceSelection,
} from '@/common/types/agentPlatform';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { AGENT_CHAT_MODEL_TASK, CHAT_ROUTE_RECORD_SCHEMA } from '@/common/types/agentPlatform';
import { Button, Collapse, Input, Select, Tag } from '@arco-design/web-react';
import { Copy, Lock } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { WorkspaceFolderSelect } from '@/renderer/components/workspace';
import {
  DEFAULT_WORKSPACE_RESOURCE_ID,
  KNOWLEDGE_NAME_PARAMETER,
  KNOWLEDGE_ROOT_PARAMETER,
  TEMPLATE_I18N_PATH,
  WORKSPACE_ROOT_PARAMETER,
} from './model';
import styles from './AgentSettingsPage.module.css';

type OfficialTemplateOverviewProps = {
  template: OfficialPresetTemplate;
  busy: boolean;
  hostWorkDir: string | null;
  knowledgeBases: IKnowledgeBase[];
  knowledgeBasesLoading: boolean;
  connectors: IMcpServer[];
  onFork: (
    displayName: string,
    resources: TemplateResourceSelection[],
    modelRouteRefs: Record<string, string>,
    chatRouteRecords: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>
  ) => void;
};

const ExactRefList: React.FC<{
  title: string;
  items: Array<{ id: string; version: string }>;
  emptyLabel: string;
}> = ({ title, items, emptyLabel }) => (
  <div className={styles.templateColumn}>
    <div className={styles.templateColumnHeader}>
      <span>{title}</span>
      <span>{items.length}</span>
    </div>
    {items.length === 0 ? (
      <div className={styles.inlineEmpty}>{emptyLabel}</div>
    ) : (
      <div className={styles.exactList}>
        {items.map((item) => (
          <div key={`${item.id}@${item.version}`} className={styles.exactRow}>
            <span>{item.id}</span>
            <code>{item.version}</code>
          </div>
        ))}
      </div>
    )}
  </div>
);

const OfficialTemplateOverview: React.FC<OfficialTemplateOverviewProps> = ({
  template,
  busy,
  hostWorkDir,
  knowledgeBases,
  knowledgeBasesLoading,
  connectors,
  onFork,
}) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const [resourceIds, setResourceIds] = useState<Record<string, string>>({});
  const [workspaceRoots, setWorkspaceRoots] = useState<Record<string, string>>({});
  const [chatRouteRecordText, setChatRouteRecordText] = useState('');
  const [chatRouteRecord, setChatRouteRecord] = useState<ChatRouteRecord | null>(null);

  useEffect(() => {
    setResourceIds(
      template.template_key === 'coding.codex' && hostWorkDir
        ? { workspace: DEFAULT_WORKSPACE_RESOURCE_ID }
        : {}
    );
    setWorkspaceRoots(hostWorkDir ? { workspace: hostWorkDir } : {});
    setChatRouteRecordText('');
    setChatRouteRecord(null);
  }, [hostWorkDir, template.template_key]);

  const resources = useMemo(
    () =>
      template.seed.typed_resource_defaults
        .map((resource): TemplateResourceSelection | null => {
          const workspaceRoot =
            workspaceRoots[resource.slot_key] ??
            (resource.resource_kind === 'workspace' ? hostWorkDir : null);
          const resourceId =
            resourceIds[resource.slot_key] ??
            (resource.resource_kind === 'workspace' && workspaceRoot
              ? DEFAULT_WORKSPACE_RESOURCE_ID
              : '');
          if (!resourceId) return null;
          const knowledgeBase =
            resource.resource_kind === 'knowledge_base'
              ? knowledgeBases.find(
                  (candidate) => String(candidate.knowledge_base_id) === String(resourceId)
                )
              : undefined;
          return {
            slot_key: resource.slot_key,
            resource_kind: resource.resource_kind,
            resource_id: resourceId,
            typed_parameters:
              resource.resource_kind === 'workspace' && workspaceRoot
                ? { [WORKSPACE_ROOT_PARAMETER]: workspaceRoot }
                : knowledgeBase
                  ? {
                      [KNOWLEDGE_ROOT_PARAMETER]: knowledgeBase.root_path,
                      [KNOWLEDGE_NAME_PARAMETER]: knowledgeBase.name,
                    }
                  : {},
          };
        })
        .filter((resource): resource is TemplateResourceSelection => resource !== null),
    [
      hostWorkDir,
      knowledgeBases,
      resourceIds,
      template.seed.typed_resource_defaults,
      workspaceRoots,
    ]
  );
  const missingRequired = template.seed.typed_resource_defaults.some(
    (resource) =>
      resource.required &&
      !(
        resourceIds[resource.slot_key]?.trim() ||
        (resource.resource_kind === 'workspace' &&
          (workspaceRoots[resource.slot_key] || hostWorkDir))
      )
  );
  const resourceLabelFor = (resourceKind: string): string => {
    switch (resourceKind) {
      case 'workspace':
        return t('terminal.create.workspace');
      case 'knowledge_base':
        return t('agentSettings.resources.knowledgeBase');
      case 'mcp_server':
        return t('agentSettings.sections.mcp');
      case 'companion':
      case 'companion_memory':
        return t('agentSettings.template.companion.default.name');
      case 'channel':
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
      default:
        return t('agentSettings.sections.resources');
    }
  };
  const updateWorkspaceSelection = (slotKey: string, workspaceRoot: string) => {
    setWorkspaceRoots((current) => {
      const next = { ...current };
      if (workspaceRoot) next[slotKey] = workspaceRoot;
      else delete next[slotKey];
      return next;
    });
    setResourceIds((current) => {
      const next = { ...current };
      if (workspaceRoot) next[slotKey] = DEFAULT_WORKSPACE_RESOURCE_ID;
      else delete next[slotKey];
      return next;
    });
  };
  const updateChatRouteRecord = (text: string) => {
    setChatRouteRecordText(text);
    if (!text.trim()) {
      setChatRouteRecord(null);
      return;
    }
    try {
      const parsed = JSON.parse(text) as ChatRouteRecord;
      if (
        parsed.schema === CHAT_ROUTE_RECORD_SCHEMA &&
        parsed.task === AGENT_CHAT_MODEL_TASK &&
        parsed.primary?.model_route_id
      ) {
        setChatRouteRecord(parsed);
      }
    } catch {
      setChatRouteRecord(null);
    }
  };

  return (
    <main className={styles.editorSurface}>
      <header className={styles.editorHeader}>
        <div className={styles.editorHeaderCopy}>
          <div className={styles.eyebrow}>
            <Lock theme='outline' size='14' />
            {t('agentSettings.template.readOnly')}
          </div>
          <h2>{name}</h2>
          <p>{t(`agentSettings.template.${path}.description`)}</p>
        </div>
        <Button
          type='primary'
          icon={<Copy theme='outline' size='15' />}
          loading={busy}
          disabled={missingRequired}
          onClick={() =>
            onFork(
              t('agentSettings.defaults.forkName', { name }),
              resources,
              chatRouteRecord
                ? {
                    [AGENT_CHAT_MODEL_TASK]: chatRouteRecord.primary.model_route_id,
                  }
                : {},
              chatRouteRecord ? { [AGENT_CHAT_MODEL_TASK]: chatRouteRecord } : {}
            )
          }
        >
          {t('agentSettings.actions.fork')}
        </Button>
      </header>

      <section className={styles.section}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.capabilities')}</h3>
            <p>{t('agentSettings.template.capabilityHint')}</p>
          </div>
        </div>
        <div className={styles.enabledSummary}>
          <span>{t('agentSettings.sections.capabilities')}</span>
          <strong>
            {template.seed.initial_capabilities.length +
              template.seed.on_demand_capabilities.length}
          </strong>
        </div>
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.resources')}</h3>
            <p>{t('agentSettings.template.resourceHint')}</p>
          </div>
        </div>
        {template.seed.typed_resource_defaults.length === 0 ? (
          <div className={styles.inlineEmpty}>{t('agentSettings.resources.noneRequired')}</div>
        ) : (
          <div className={styles.resourceDefaults}>
            {template.seed.typed_resource_defaults.map((resource) => (
              <div key={resource.slot_key} className={styles.resourceDefaultRow}>
                <div>
                  <strong>{resourceLabelFor(resource.resource_kind)}</strong>
                  <span>
                    {resource.required
                      ? t('agentSettings.resources.required')
                      : t('agentSettings.resources.optional')}
                  </span>
                </div>
                <div className={styles.resourcePicker}>
                  {resource.resource_kind === 'workspace' && (
                    <WorkspaceFolderSelect
                      value={workspaceRoots[resource.slot_key] ?? hostWorkDir ?? ''}
                      onChange={(workspaceRoot: string) =>
                        updateWorkspaceSelection(resource.slot_key, workspaceRoot)
                      }
                      onClear={() => updateWorkspaceSelection(resource.slot_key, '')}
                      placeholder={t('terminal.create.workspacePlaceholder')}
                      recentLabel={t('terminal.create.recent')}
                      chooseDifferentLabel={t('terminal.create.chooseFolder')}
                    />
                  )}
                  {resource.resource_kind === 'knowledge_base' && (
                    <Select
                      value={resourceIds[resource.slot_key] || undefined}
                      loading={knowledgeBasesLoading}
                      showSearch
                      placeholder={t('agentSettings.resources.knowledgeBasePlaceholder')}
                      options={knowledgeBases.map((knowledgeBase) => ({
                        label: knowledgeBase.name,
                        value: knowledgeBase.knowledge_base_id,
                        disabled: !knowledgeBase.root_exists,
                      }))}
                      onChange={(knowledgeBaseId: string) =>
                        setResourceIds((current) => ({
                          ...current,
                          [resource.slot_key]: knowledgeBaseId,
                        }))
                      }
                    />
                  )}
                  {resource.resource_kind === 'mcp_server' && (
                    <Select
                      value={resourceIds[resource.slot_key] || undefined}
                      disabled={connectors.length === 0}
                      showSearch
                      placeholder={t('common.select')}
                      options={connectors.map((connector) => ({
                        label: connector.name,
                        value: String(connector.mcp_server_id),
                      }))}
                      onChange={(connectorId: string) =>
                        setResourceIds((current) => ({
                          ...current,
                          [resource.slot_key]: connectorId,
                        }))
                      }
                    />
                  )}
                  {!['workspace', 'knowledge_base', 'mcp_server'].includes(
                    resource.resource_kind
                  ) && (
                    <Tag size='small' color='orange'>
                      {resource.required
                        ? t('agentSettings.common.unavailable')
                        : t('agentSettings.common.none')}
                    </Tag>
                  )}
                </div>
                <div className={styles.tagRow}>
                  <Tag size='small' color={resource.required ? 'red' : 'gray'}>
                    {resource.required
                      ? t('agentSettings.resources.required')
                      : t('agentSettings.resources.optional')}
                  </Tag>
                </div>
              </div>
            ))}
          </div>
        )}
        <Collapse defaultActiveKey={[]} className={styles.technicalCollapse}>
          <Collapse.Item name='template-technical-details' header={t('common.technical_details')}>
            <label className={styles.field}>
              <span>{t('agentSettings.fields.chatModelRouteRecord')}</span>
              <Input.TextArea
                value={chatRouteRecordText}
                placeholder={t('agentSettings.fields.chatModelRouteRecordPlaceholder')}
                autoSize={{ minRows: 4, maxRows: 12 }}
                onChange={updateChatRouteRecord}
              />
            </label>
            <div className={styles.dualGrid}>
              <ExactRefList
                title={t('agentSettings.capabilities.initial')}
                items={template.seed.initial_capabilities}
                emptyLabel={t('agentSettings.common.none')}
              />
              <ExactRefList
                title={t('agentSettings.capabilities.onDemand')}
                items={template.seed.on_demand_capabilities}
                emptyLabel={t('agentSettings.common.none')}
              />
            </div>
            <div className={styles.tagRow}>
              {template.role_coverage.required_capability_categories.map((category) => (
                <Tag key={category} size='small' color='gray'>
                  {category}
                </Tag>
              ))}
            </div>
          </Collapse.Item>
        </Collapse>
      </section>

      {template.template_key === 'chat.minimal' && (
        <section className={styles.zeroToolBand}>
          <strong>{t('agentSettings.template.zeroToolTitle')}</strong>
          <span>{t('agentSettings.template.zeroToolBody')}</span>
        </section>
      )}
    </main>
  );
};

export default OfficialTemplateOverview;
