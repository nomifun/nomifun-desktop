import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { Refresh } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useKnowledgeBases } from '@/renderer/pages/knowledge/useKnowledge';
import AgentPresetEditor from './AgentPresetEditor';
import AgentPresetLibrary from './AgentPresetLibrary';
import OfficialTemplateOverview from './OfficialTemplateOverview';
import { useAgentSettingsController } from './useAgentSettingsController';
import styles from './AgentSettingsPage.module.css';

const AgentSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const controller = useAgentSettingsController();
  const { bases: knowledgeBases, loading: knowledgeBasesLoading } = useKnowledgeBases();
  const sourceTemplate =
    controller.draft?.source_template_key == null
      ? undefined
      : controller.library?.official_templates.find(
          (template) => template.template_key === controller.draft?.source_template_key
        );
  const selectedTemplate =
    controller.selection?.kind === 'template' ? controller.selection.template : null;

  return (
    <HubPageShell
      title={t('agentSettings.title')}
      maxWidthClass='md:max-w-1440px'
      className={styles.pageShell}
    >
      {controller.error && (
        <Alert
          type='error'
          showIcon
          title={t('agentSettings.errors.title')}
          content={
            <div className={styles.errorBody}>
              <span>{controller.error}</span>
              <Button
                type='text'
                size='mini'
                icon={<Refresh theme='outline' size='14' />}
                onClick={() => void controller.load()}
              >
                {t('agentSettings.actions.retry')}
              </Button>
            </div>
          }
          className={styles.pageError}
        />
      )}

      {controller.loading && !controller.library ? (
        <div className={styles.loading}>
          <Spin size={24} />
          <span>{t('agentSettings.loading')}</span>
        </div>
      ) : controller.library ? (
        <div className={styles.workspace}>
          <AgentPresetLibrary
            library={controller.library}
            selection={controller.selection}
            busy={controller.busyAction === 'create' || controller.busyAction === 'fork'}
            onSelectTemplate={controller.openTemplate}
            onSelectPreset={(preset) => void controller.openPreset(preset)}
            onCreatePreset={(displayName) => void controller.createPreset(displayName)}
          />

          {selectedTemplate ? (
            <OfficialTemplateOverview
              template={selectedTemplate}
              busy={controller.busyAction === 'fork'}
              hostWorkDir={controller.hostWorkDir}
              knowledgeBases={knowledgeBases}
              knowledgeBasesLoading={knowledgeBasesLoading}
              connectors={controller.connectors}
              onFork={(displayName, resources, modelRoutes, routeRecords) =>
                void controller.forkTemplate(
                  selectedTemplate.template_key,
                  displayName,
                  resources,
                  modelRoutes,
                  routeRecords
                )
              }
            />
          ) : controller.editor && controller.draft ? (
            <AgentPresetEditor
              editor={controller.editor}
              draft={controller.draft}
              catalog={controller.catalog}
              preview={controller.preview}
              testResult={controller.testResult}
              tokenState={controller.tokenState}
              hostWorkDir={controller.hostWorkDir}
              connectors={controller.connectors}
              knowledgeBases={knowledgeBases}
              knowledgeBasesLoading={knowledgeBasesLoading}
              sourceTemplate={sourceTemplate}
              busyAction={controller.busyAction}
              onDraftChange={controller.setDraft}
              onPreview={() => void controller.runPreview()}
              onSave={() => void controller.saveRevision()}
              onTest={(input) => void controller.runTest(input)}
            />
          ) : (
            <div className={styles.loading}>
              <Spin size={20} />
              <span>{t('agentSettings.loadingEditor')}</span>
            </div>
          )}
        </div>
      ) : null}
    </HubPageShell>
  );
};

export default AgentSettingsPage;
