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
import { agentEditorReturn, editingDocument, type AgentEditorReturn } from './model';
import AgentPresetEditor from './AgentPresetEditor';
import GuidDefaultAgentSetting from './GuidDefaultAgentSetting';
import AgentRoleDefaults from './AgentRoleDefaults';
import AgentPresetLibrary from './AgentPresetLibrary';
import { useAgentSettingsController } from './useAgentSettingsController';
import { useAgentWorkbenchEntry } from './useAgentWorkbenchEntry';
import styles from './AgentSettingsPage.module.css';

const AgentSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const navigationHistory = useNavigationHistory();
  const [returned] = useState(() => agentEditorReturn(location.state, location.search));
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
    // Portaled settings dialogs manage their own keyboard focus.
    if (!event.currentTarget.contains(event.target as Node)) return;
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
  const [pendingSwitch, setPendingSwitch] = useState<(() => void) | null>(null);
  const hasUnsavedChanges = controller.dirty;
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
  useAgentWorkbenchEntry({ ...controller, loading: controller.loading || (!!returned && location.search === returned.search),
    rememberedSelection: controller.editingDrafts?.readSelection() });
  useEffect(() => {
    if (!returned || restored.current || controller.loading || !controller.library) return;
    restored.current = true;
    const preset = [...controller.library.user_presets, ...Object.values(controller.library.official_presets)]
      .find(value => value.preset_id === returned.draft.preset_id);
    if (preset) void controller.openPreset(preset, returned.draft);
  }, [returned, controller.loading, controller.library, controller.openPreset]);
  const sourceTemplate =
    controller.draft?.source_template_key == null
      ? undefined
      : controller.library?.official_templates.find(
          (template) => template.template_key === controller.draft?.source_template_key
        );
  const selectedTemplate =
    controller.selection?.kind === 'template' ? controller.selection.template : null;
  const openKeepingEdits = async (destination: string) => {
    const params = new URLSearchParams(location.search);
    let snapshot: AgentEditorReturn | null = null;
    if (controller.draft) {
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
    const templateKey = preset.source === 'official'
      ? Object.entries(controller.library?.official_presets ?? {})
        .find(([, official]) => official.preset_id === preset.preset_id)?.[0]
      : undefined;
    const productPath = templateKey === 'companion.default' ? '/nomi'
      : templateKey === 'customer-service.default' ? '/customer-service' : undefined;
    if (productPath) {
      void navigate(productPath);
      return;
    }
    void navigate('/guid', {
      state: templateKey ? { selectedAgentTemplateKey: templateKey } : { selectedAgentPresetId: preset.preset_id },
    });
  };

  const visibleLibrary = useMemo(() => {
    const currentDraft = controller.draft;
    if (!controller.library || !currentDraft) return controller.library;
    const draftSummary = (preset: AgentPresetSummary) => preset.preset_id === currentDraft.preset_id
      ? {
          ...preset,
          display_name: currentDraft.display_name.trim() || t('agentSettings.defaults.untitledName'),
          description: currentDraft.description,
        }
      : preset;
    return {
      ...controller.library,
      official_presets: Object.fromEntries(Object.entries(controller.library.official_presets)
        .map(([key, preset]) => [key, draftSummary(preset)])),
      user_presets: controller.library.user_presets.map(draftSummary),
    };
  }, [controller.draft, controller.library, t]);

  const libraryPanel = controller.library && visibleLibrary ? (<AgentPresetLibrary
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
            footer={<div className={styles.libraryFooter}>
              <GuidDefaultAgentSetting library={controller.library} />
              <AgentRoleDefaults catalog={controller.catalog} />
            </div>}
            onSelectTemplate={(template) => {
              if (controller.selection?.kind === 'template' && controller.selection.template.template_key === template.template_key) return;
              if (controller.selection?.kind === 'preset' && controller.selection.preset.preset_id === controller.library?.official_presets?.[template.template_key]?.preset_id) return;
              beforeSwitch(() => { void controller.openTemplate(template); setNarrowSiderOpen(false); });
            }}
            onSelectPreset={(preset) => {
              if (controller.selection?.kind === 'preset' && controller.selection.preset.preset_id === preset.preset_id) return;
              beforeSwitch(() => { void controller.openPreset(preset); setNarrowSiderOpen(false); });
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
        onOk={() => {
          controller.discardChanges();
          pendingSwitch?.(); setPendingSwitch(null);
        }}
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
          {selectedTemplate && controller.busyAction === 'open' ? (
            <div className={styles.loading} role='status'>
              <Spin size={24} />
              <span>{t('agentSettings.loading')}</span>
            </div>
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
              onCopy={controller.editor.preset.source === 'official' ? () => {
                if (!controller.draft) return;
                void controller.createConfiguredPreset(controller.draft.display_name, controller.draft.document, controller.draft.description);
              } : undefined}
              onDiscard={controller.discardChanges}
              onOpenModels={() => { void openKeepingEdits('/models'); }}
              onOpenAuthor={openKeepingEdits}
              startConversationLabel={controller.editor.preset.source === 'official'
                ? sourceTemplate?.template_key === 'companion.default' ? t('agentSettings.workbench.openCompanions')
                  : sourceTemplate?.template_key === 'customer-service.default' ? t('agentSettings.workbench.openCustomerService') : undefined
                : undefined}
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
