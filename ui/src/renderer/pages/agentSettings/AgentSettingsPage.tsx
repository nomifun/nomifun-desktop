import { useContentSiderCollapse } from '@/renderer/components/layout/ContentSider';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import { AGENT_SIDER_TOGGLE_EVENT, dispatchAgentSiderStateEvent } from '@/renderer/utils/workspace/agentSiderEvents';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import { Alert, Button, Modal, Spin } from '@arco-design/web-react';
import { AddOne, Refresh } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useNavigationHistory } from '@/renderer/hooks/context/NavigationHistoryContext';
import { agentEditorReturn, editingDocument, type AgentEditorReturn, type TemplateEditingState } from './model';
import AgentPresetEditor from './AgentPresetEditor';
import AgentRoleDefaults from './AgentRoleDefaults';
import AgentPresetLibrary from './AgentPresetLibrary';
import OfficialTemplateOverview from './OfficialTemplateOverview';
import { useAgentSettingsController } from './useAgentSettingsController';
import { useAgentWorkbenchEntry } from './useAgentWorkbenchEntry';
import styles from './AgentSettingsPage.module.css';

const AgentSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const navigationHistory = useNavigationHistory();
  const [returned] = useState(() => agentEditorReturn(location.state, location.search));
  const [templateEditing, setTemplateEditing] = useState<TemplateEditingState | null>(null);
  const [templateInitialEditing, setTemplateInitialEditing] = useState(returned?.kind === 'template' ? returned.editing : undefined);
  const restored = useRef(false);
  useEffect(() => {
    if (!returned) return;
    const { agentEditorReturn: _consumed, ...state } = location.state ?? {};
    const path = `${location.pathname}${location.search}`;
    if (navigationHistory) navigationHistory.replaceCurrent(path, state);
    else void navigate(path, { replace: true, state });
  }, []);
  const desktopSider = useContentSiderCollapse('nomifun:agent-sider-collapsed', false);
  const resize = useResizableSplit({ unit: 'px', defaultWidth: 300, minWidth: 240, maxWidth: 480, storageKey: 'nomifun:agent-sider-width' });
  const [narrow, setNarrow] = useState(() => typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 1250px)').matches);
  const [narrowSiderOpen, setNarrowSiderOpen] = useState(false);
  const narrowOverlay = useRef<HTMLDivElement>(null);
  const narrowReturnFocus = useRef<HTMLElement | null>(null);
  const collapsed = narrow ? !narrowSiderOpen : desktopSider.collapsed;
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(max-width: 1250px)');
    const update = () => { setNarrow(media.matches); setNarrowSiderOpen(false); };
    media.addEventListener('change', update);

  return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => { dispatchAgentSiderStateEvent(collapsed); }, [collapsed]);
  useEffect(() => {
    if (!narrow || !narrowSiderOpen) return;
    const frame = requestAnimationFrame(() => {
      narrowOverlay.current?.querySelector<HTMLElement>('input, button, [tabindex="0"]')?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [narrow, narrowSiderOpen]);
  const closeNarrowSider = () => {
    setNarrowSiderOpen(false);
    requestAnimationFrame(() => narrowReturnFocus.current?.focus());
  };
  useEffect(() => {
    const toggle = () => {
      if (!narrow) { desktopSider.toggle(); return; }
      setNarrowSiderOpen((open) => {
        if (!open) narrowReturnFocus.current = document.activeElement as HTMLElement | null;
        else requestAnimationFrame(() => narrowReturnFocus.current?.focus());
        return !open;
      });
    };
    window.addEventListener(AGENT_SIDER_TOGGLE_EVENT, toggle);
    return () => window.removeEventListener(AGENT_SIDER_TOGGLE_EVENT, toggle);
  }, [narrow, desktopSider.toggle]);
  const collapse = () => narrow ? closeNarrowSider() : desktopSider.setCollapsed(true);
  const trapNarrowOverlay = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeNarrowSider();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = [...(narrowOverlay.current?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), [href], [tabindex]:not([tabindex="-1"])'
    ) ?? [])];
    if (!focusable.length) return;
    const first = focusable[0], last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault(); last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault(); first.focus();
    }
  };
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
  useAgentWorkbenchEntry({ ...controller, loading: controller.loading || (!!returned && location.search === returned.search) });
  useEffect(() => {
    if (!returned || restored.current || controller.loading || !controller.library) return;
    restored.current = true;
    if (returned.kind === 'template') {
      const template = controller.library.official_templates.find(value => value.template_key === returned.templateKey);
      if (template) controller.openTemplate(template);
    } else {
      const preset = controller.library.user_presets.find(value => value.preset_id === returned.draft.preset_id);
      if (preset) void controller.openPreset(preset, returned.draft);
    }
  }, [returned, controller.loading, controller.library, controller.openTemplate, controller.openPreset]);
  const sourceTemplate =
    controller.draft?.source_template_key == null
      ? undefined
      : controller.library?.official_templates.find(
          (template) => template.template_key === controller.draft?.source_template_key
        );
  const selectedTemplate =
    controller.selection?.kind === 'template' ? controller.selection.template : null;
  const previousTemplate = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (returned?.kind === 'template' && previousTemplate.current === returned.templateKey && selectedTemplate?.template_key !== returned.templateKey) {
      setTemplateInitialEditing(undefined);
    }
    previousTemplate.current = selectedTemplate?.template_key;
  }, [selectedTemplate?.template_key, returned]);
  const openKeepingEdits = async (destination: string) => {
    const params = new URLSearchParams(location.search);
    let snapshot: AgentEditorReturn | null = null;
    if (selectedTemplate && templateEditing) {
      params.delete('preset'); params.set('template', selectedTemplate.template_key);
      snapshot = { version: 1, search: `?${params}`, kind: 'template', templateKey: selectedTemplate.template_key,
        editing: templateEditing };
    } else if (controller.draft) {
      params.delete('template'); params.set('preset', controller.draft.preset_id);
      snapshot = { version: 1, search: `?${params}`, kind: 'preset', draft: {
        ...controller.draft, document: editingDocument(controller.draft.document),
      } };
    }
    if (snapshot) {
      const path = `/agent${snapshot.search}`, state = { agentEditorReturn: snapshot };
      if (navigationHistory) navigationHistory.replaceCurrent(path, state);
      else await navigate(path, { replace: true, state });
    }
    await navigate(destination);
  };
  const startConversation = (preset: AgentPresetSummary) => {
    void navigate('/guid', {
      state: {
        selectedAgentPresetId: preset.preset_id,
      },
    });
  };

  const visibleLibrary = useMemo(() => {
    const currentDraft = controller.draft;
    if (!controller.library || !currentDraft) return controller.library;
    return {
      ...controller.library,
      user_presets: controller.library.user_presets.map((preset) =>
        preset.preset_id === currentDraft.preset_id
          ? {
              ...preset,
              display_name: currentDraft.display_name.trim() || t('agentSettings.defaults.untitledName'),
              description: currentDraft.description,
            }
          : preset
      ),
    };
  }, [controller.draft, controller.library, t]);

  const libraryPanel = visibleLibrary ? (<AgentPresetLibrary
            width={narrow ? 300 : resize.splitRatio}
            resizeHandle={narrow ? undefined : resize.createDragHandle({ className: 'right-0' })}
            onCollapse={collapse}
            library={visibleLibrary}
            selection={controller.selection}
            dirtyPresetId={controller.dirty ? controller.draft?.preset_id : undefined}
            busy={controller.busyAction !== null}
            creating={controller.busyAction === 'create'}
            openingPresetId={controller.openingPresetId}
            deletingPresetId={controller.deletingPresetId}
            onSelectTemplate={(template) => {
              if (controller.selection?.kind === 'template' && controller.selection.template.template_key === template.template_key) return;
              beforeSwitch(() => { setTemplateInitialEditing(undefined); controller.openTemplate(template); setNarrowSiderOpen(false); });
            }}
            onSelectPreset={(preset) => {
              if (controller.selection?.kind === 'preset' && controller.selection.preset.preset_id === preset.preset_id) return;
              beforeSwitch(() => { setTemplateInitialEditing(undefined); void controller.openPreset(preset); setNarrowSiderOpen(false); });
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
              {controller.errorSubjects.length > 0 && (
                <span>{t('agentSettings.errors.diagnosticSubjects', {
                  subjects: controller.errorSubjects.join(', '),
                })}</span>
              )}
              {controller.modelConfigurationMissing && <div>
                <Button size='small' type='primary' disabled={controller.busyAction !== null}
                  onClick={() => void openKeepingEdits('/models')}>{t('agentSettings.workbench.configureChatModel')}</Button>
                <p>{t('agentSettings.workbench.configureModelReturn')}</p>
              </div>}
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
          {!collapsed && (narrow ? <div ref={narrowOverlay} className={styles.siderOverlay} role='dialog' aria-modal='true' aria-label={t('agentSettings.library.ariaLabel')} onKeyDown={trapNarrowOverlay}><button className={styles.siderBackdrop} aria-label={t('agentSettings.workbench.hideList')} onClick={collapse} />{libraryPanel}</div> : libraryPanel)}
          <div className={styles.mainArea}>
          <div className={styles.defaultsToolbar}><AgentRoleDefaults catalog={controller.catalog} /></div>

          {selectedTemplate ? (
            <OfficialTemplateOverview
              key={selectedTemplate.template_key}
              template={selectedTemplate}
              busy={controller.busyAction !== null}
              catalog={controller.catalog}
              onDirtyChange={setTemplateDirty}
              initialEditing={returned?.kind === 'template' && selectedTemplate.template_key === returned.templateKey ? templateInitialEditing : undefined}
              onEditingChange={setTemplateEditing}
              onSave={(displayName, document, description) => { void controller.createConfiguredPreset(displayName, document, description); }}
            />
          ) : controller.editor && controller.draft ? (
            <AgentPresetEditor
              key={controller.editor.preset.preset_id}
              editor={controller.editor}
              draft={controller.draft}
              catalog={controller.catalog}
              sourceTemplate={sourceTemplate}
              busyAction={controller.busyAction}
              dirty={controller.dirty}
              onDraftChange={controller.setDraft}
              onSave={() => void controller.saveRevision()}
              onDiscard={controller.discardChanges}
              onOpenModels={() => { void openKeepingEdits('/models'); }}
              onOpenAuthor={openKeepingEdits}
              onStartConversation={startConversation}
            />
          ) : (
            <div className={styles.workbenchEmpty} role='status'>
              <span className={styles.workbenchEmptyIcon}><AddOne theme='outline' size={24} /></span>
              <h2>{t('agentSettings.workbench.emptyWorkbenchTitle')}</h2>
              <p>{t('agentSettings.workbench.emptyWorkbenchHint')}</p>
              <Button
                type='primary'
                disabled={controller.busyAction !== null}
                loading={controller.busyAction === 'create'}
                onClick={() => void controller.createPreset(t('agentSettings.defaults.untitledName'))}
              >
                {t('agentSettings.workbench.startCustom')}
              </Button>
            </div>
          )}
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default AgentSettingsPage;
