import { useContentSiderCollapse } from '@/renderer/components/layout/ContentSider';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import { AGENT_SIDER_TOGGLE_EVENT, dispatchAgentSiderStateEvent } from '@/renderer/utils/workspace/agentSiderEvents';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import { Alert, Button, Modal, Spin } from '@arco-design/web-react';
import { Refresh } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
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
  const desktopSider = useContentSiderCollapse('nomifun:agent-sider-collapsed', false);
  const resize = useResizableSplit({ unit: 'px', defaultWidth: 300, minWidth: 240, maxWidth: 480, storageKey: 'nomifun:agent-sider-width' });
  const [narrow, setNarrow] = useState(() => typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 1250px)').matches);
  const [mobileOpen, setMobileOpen] = useState(false);
  const collapsed = narrow ? !mobileOpen : desktopSider.collapsed;
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(max-width: 1250px)');
    const update = () => { setNarrow(media.matches); setMobileOpen(false); };
    media.addEventListener('change', update);

  return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => { dispatchAgentSiderStateEvent(collapsed); }, [collapsed]);
  useEffect(() => {
    const toggle = () => narrow ? setMobileOpen(open => !open) : desktopSider.toggle();
    window.addEventListener(AGENT_SIDER_TOGGLE_EVENT, toggle);
    return () => window.removeEventListener(AGENT_SIDER_TOGGLE_EVENT, toggle);
  }, [narrow, desktopSider.toggle]);
  const collapse = () => narrow ? setMobileOpen(false) : desktopSider.setCollapsed(true);
  const controller = useAgentSettingsController();
  const [templateDirty, setTemplateDirty] = useState(false);
  const [pendingSwitch, setPendingSwitch] = useState<(() => void) | null>(null);
  const hasUnsavedChanges = controller.dirty || templateDirty;
  useEffect(() => {
    if (!hasUnsavedChanges) return;
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [hasUnsavedChanges]);
  const beforeSwitch = (action: () => void) => {
    if (!hasUnsavedChanges) { action(); return; }
    setPendingSwitch(() => action);
  };
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

  const libraryPanel = controller.library ? (<AgentPresetLibrary
            width={narrow ? 300 : resize.splitRatio}
            resizeHandle={narrow ? undefined : resize.createDragHandle({ className: 'right-0' })}
            onCollapse={collapse}
            library={controller.library}
            selection={controller.selection}
            busy={controller.busyAction !== null}
            creating={controller.busyAction === 'create'}
            openingPresetId={controller.openingPresetId}
            deletingPresetId={controller.deletingPresetId}
            onSelectTemplate={(template) => {
              if (controller.selection?.kind === 'template' && controller.selection.template.template_key === template.template_key) return;
              beforeSwitch(() => { controller.openTemplate(template); setMobileOpen(false); });
            }}
            onSelectPreset={(preset) => {
              if (controller.selection?.kind === 'preset' && controller.selection.preset.preset_id === preset.preset_id) return;
              beforeSwitch(() => { void controller.openPreset(preset); setMobileOpen(false); });
            }}
            onCreatePreset={(displayName) => beforeSwitch(() => { void controller.createPreset(displayName); })}
            onDeletePreset={(preset) => controller.deletePreset(preset)}
          />) : null;

  return (
    <div className={styles.pageShell}>
      <Modal
        visible={pendingSwitch !== null}
        title={t('agentSettings.workbench.leaveTitle')}
        okText={t('agentSettings.workbench.discardAndLeave')}
        cancelText={t('agentSettings.workbench.keepEditing')}
        onCancel={() => setPendingSwitch(null)}
        onOk={() => { pendingSwitch?.(); setPendingSwitch(null); }}
        autoFocus
        focusLock
      >
        {t('agentSettings.workbench.leaveBody')}
      </Modal>
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
          {!collapsed && (narrow ? <div className={styles.siderOverlay}><button className={styles.siderBackdrop} aria-label={t('agentSettings.workbench.hideList')} onClick={collapse} />{libraryPanel}</div> : libraryPanel)}
          <div className={styles.mainArea}>

          {selectedTemplate ? (
            <OfficialTemplateOverview
              key={selectedTemplate.template_key}
              template={selectedTemplate}
              busy={controller.busyAction !== null}
              catalog={controller.catalog}
              onDirtyChange={setTemplateDirty}
              onSave={(displayName, document, description) => { void controller.createConfiguredPreset(displayName, document, description); }}
            />
          ) : controller.editor && controller.draft ? (
            <AgentPresetEditor
              key={controller.editor.preset.preset_id}
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
              onDiscard={controller.discardChanges}
              onOpenModels={() => beforeSwitch(() => { void navigate('/models'); })}
              onOpenResource={(route) => beforeSwitch(() => { void navigate(route); })}
              onTest={(input, resourceSelections) => void controller.runTest(input, resourceSelections)}
              onStartConversation={startConversation}
            />
          ) : (
            <div className={styles.loading}>
              <Spin size={20} />
              <span>{t('agentSettings.loadingEditor')}</span>
            </div>
          )}
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default AgentSettingsPage;
