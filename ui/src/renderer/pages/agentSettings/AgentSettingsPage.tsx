import HubPageShell from '@/renderer/components/layout/HubPageShell';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { Refresh } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import AgentPresetEditor from './AgentPresetEditor';
import AgentPresetLibrary from './AgentPresetLibrary';
import OfficialTemplateOverview from './OfficialTemplateOverview';
import { useAgentSettingsController } from './useAgentSettingsController';
import { useAgentWorkbenchEntry } from './useAgentWorkbenchEntry';
import styles from './AgentSettingsPage.module.css';

const AgentSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const controller = useAgentSettingsController();
  useAgentWorkbenchEntry(controller);
  const sourceTemplate =
    controller.draft?.source_template_key == null
      ? undefined
      : controller.library?.official_templates.find(
          (template) => template.template_key === controller.draft?.source_template_key
        );
  const selectedTemplate =
    controller.selection?.kind === 'template' ? controller.selection.template : null;
  const startConversation = (preset: AgentPresetSummary) => {
    void navigate('/guid', {
      state: {
        selectedAgentPresetId: preset.preset_id,
      },
    });
  };

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
            busy={controller.busyAction !== null}
            creating={controller.busyAction === 'create'}
            openingPresetId={controller.openingPresetId}
            deletingPresetId={controller.deletingPresetId}
            onSelectTemplate={controller.openTemplate}
            onSelectPreset={(preset) => void controller.openPreset(preset)}
            onCreatePreset={(displayName) => void controller.createPreset(displayName)}
            onDeletePreset={(preset) => controller.deletePreset(preset)}
          />

          {selectedTemplate ? (
            <OfficialTemplateOverview
              template={selectedTemplate}
              busy={controller.busyAction === 'fork'}
              catalog={controller.catalog}
              onFork={(displayName, modelRoutes, routeRecords) =>
                void controller.forkTemplate(
                  selectedTemplate.template_key,
                  displayName,
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
              sourceTemplate={sourceTemplate}
              busyAction={controller.busyAction}
              dirty={controller.dirty}
              onDraftChange={controller.setDraft}
              onPreview={() => void controller.runPreview()}
              onSave={() => void controller.saveRevision()}
              onTest={(input) => void controller.runTest(input)}
              onStartConversation={startConversation}
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
