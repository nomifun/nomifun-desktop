import type {
  AgentCatalogResponse,
  ChatRouteRecord,
  OfficialPresetTemplate,
  TemplateResourceSelection,
} from '@/common/types/agentPlatform';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { AGENT_CHAT_MODEL_TASK } from '@/common/types/agentPlatform';
import { Alert, Button, Select, Tag } from '@arco-design/web-react';
import { Copy, Lock } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { WorkspaceFolderSelect } from '@/renderer/components/workspace';
import {
  DEFAULT_PROCESS_SESSION_RESOURCE_ID,
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
  catalog: AgentCatalogResponse;
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

const OfficialTemplateOverview: React.FC<OfficialTemplateOverviewProps> = ({
  template,
  busy,
  hostWorkDir,
  catalog,
  knowledgeBases,
  knowledgeBasesLoading,
  connectors,
  onFork,
}) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const unavailableTemplateCapabilities = useMemo(
    () =>
      [...template.seed.initial_capabilities, ...template.seed.on_demand_capabilities].filter(
        (reference) => {
          const item = catalog.capabilities.find(
            (candidate) =>
              candidate.capability.id === reference.id &&
              candidate.capability.version === reference.version
          );
          return item == null || item.materialization_state === 'unavailable';
        }
      ),
    [catalog.capabilities, template.seed.initial_capabilities, template.seed.on_demand_capabilities]
  );
  const templateUnavailable = unavailableTemplateCapabilities.length > 0;
  const [resourceIds, setResourceIds] = useState<Record<string, string>>({});
  const [workspaceRoots, setWorkspaceRoots] = useState<Record<string, string>>({});
  const workspaceDefaults = useMemo(
    () =>
      template.seed.typed_resource_defaults.filter(
        (resource) => resource.resource_kind === 'workspace'
      ),
    [template.seed.typed_resource_defaults]
  );
  const selectedWorkspaceRoot =
    workspaceDefaults
      .map((resource) => workspaceRoots[resource.slot_key]?.trim())
      .find((workspaceRoot): workspaceRoot is string => Boolean(workspaceRoot)) ??
    hostWorkDir?.trim() ??
    null;

  useEffect(() => {
    setResourceIds({});
    setWorkspaceRoots(
      hostWorkDir
        ? Object.fromEntries(
            workspaceDefaults.map((resource) => [resource.slot_key, hostWorkDir])
          )
        : {}
    );
  }, [hostWorkDir, template.template_key, workspaceDefaults]);

  const resources = useMemo(
    () =>
      template.seed.typed_resource_defaults
        .map((resource): TemplateResourceSelection | null => {
          const workspaceRoot =
            resource.resource_kind === 'workspace'
              ? workspaceRoots[resource.slot_key]?.trim() || hostWorkDir?.trim() || null
              : resource.resource_kind === 'process_session'
                ? selectedWorkspaceRoot
                : null;
          const resourceId =
            workspaceRoot && resource.resource_kind === 'workspace'
              ? DEFAULT_WORKSPACE_RESOURCE_ID
              : workspaceRoot && resource.resource_kind === 'process_session'
                ? DEFAULT_PROCESS_SESSION_RESOURCE_ID
                : resourceIds[resource.slot_key] ?? '';
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
              (resource.resource_kind === 'workspace' ||
                resource.resource_kind === 'process_session') &&
              workspaceRoot
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
      selectedWorkspaceRoot,
      template.seed.typed_resource_defaults,
      workspaceRoots,
    ]
  );
  const selectedResourceSlots = new Set(resources.map((resource) => resource.slot_key));
  const isResourceRequired = (resource: (typeof template.seed.typed_resource_defaults)[number]): boolean =>
    resource.binding_policy !== 'leave_unbound';
  const missingRequired = template.seed.typed_resource_defaults.some(
    (resource) => isResourceRequired(resource) && !selectedResourceSlots.has(resource.slot_key)
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
          disabled={missingRequired || templateUnavailable}
          onClick={() =>
            onFork(
              t('agentSettings.defaults.forkName', { name }),
              resources,
              {},
              {}
            )
          }
        >
          {t('agentSettings.actions.fork')}
        </Button>
      </header>
      {templateUnavailable && (
        <Alert
          className={styles.inlineNotice}
          type='warning'
          showIcon
          content={t('agentSettings.template.hostUnavailable')}
        />
      )}

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
                    {isResourceRequired(resource)
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
                  {resource.resource_kind === 'process_session' && (
                    <div className={styles.managedResource}>
                      <Tag size='small' color={selectedWorkspaceRoot ? 'green' : 'gray'}>
                        {selectedWorkspaceRoot
                          ? t('common.added')
                          : t('agentSettings.common.none')}
                      </Tag>
                      <span>
                        {isResourceRequired(resource)
                          ? t('agentSettings.resources.required')
                          : t('agentSettings.resources.optional')}
                      </span>
                    </div>
                  )}
                  {!['workspace', 'process_session', 'knowledge_base', 'mcp_server'].includes(
                    resource.resource_kind
                  ) && (
                    <Tag size='small' color='orange'>
                      {isResourceRequired(resource)
                        ? t('agentSettings.common.unavailable')
                        : t('agentSettings.common.none')}
                    </Tag>
                  )}
                </div>
                <div className={styles.tagRow}>
                  <Tag size='small' color={isResourceRequired(resource) ? 'red' : 'gray'}>
                    {isResourceRequired(resource)
                      ? t('agentSettings.resources.required')
                      : t('agentSettings.resources.optional')}
                  </Tag>
                </div>
              </div>
            ))}
          </div>
        )}
        <div className={styles.inlineNotice}>
          <span>
            {t('agentSettings.fields.chatModelRouteUnavailable', {
              defaultValue:
                'The canonical Chat model route is supplied by the host. No JSON or internal ID entry is required.',
            })}
          </span>
        </div>
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
