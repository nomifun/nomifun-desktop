import type { AgentCatalogResponse, AgentPresetDocument, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { createEmptyAgentPresetDocument } from '@/common/types/agentPlatform';
import { Button, Input } from '@arco-design/web-react';
import { BookmarkOne, Refresh, Save } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import AgentRuntimeEngineSelector from './AgentRuntimeEngineSelector';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import { unavailableCapabilityReferences } from './capabilityGroups';
import { TEMPLATE_I18N_PATH, editingDocument, type TemplateEditingState } from './model';
import styles from './AgentSettingsPage.module.css';

type Props = {
  template: OfficialPresetTemplate;
  busy: boolean;
  catalog: AgentCatalogResponse;
  onSave: (displayName: string, document: AgentPresetDocument, description: string) => void;
  onDirtyChange?: (dirty: boolean) => void;
  initialEditing?: TemplateEditingState;
  onEditingChange?: (editing: TemplateEditingState) => void;
};

export const documentFromTemplate = (template: OfficialPresetTemplate): AgentPresetDocument => ({
  ...createEmptyAgentPresetDocument(),
  enabled_capabilities: template.seed.enabled_capabilities.map((capability) => ({ capability, action_allowlist: [] })),

  skill_bindings: structuredClone(template.seed.skill_bindings),
});

const OfficialTemplateOverview: React.FC<Props> = ({ template, busy, catalog, onSave, onDirtyChange, initialEditing, onEditingChange }) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const original = useMemo(() => documentFromTemplate(template), [template]);
  const [document, setDocument] = useState<AgentPresetDocument>(() => ({ ...original, ...initialEditing?.document }));
  const [displayName, setDisplayName] = useState(initialEditing?.displayName ?? name);
  const [activeTab, setActiveTab] = useState(initialEditing?.activeTab ?? 'capabilities');
  useEffect(() => { onEditingChange?.({ displayName, document: editingDocument(document), activeTab }); },
    [displayName, document, activeTab, onEditingChange]);
  const tabs = [
    { key: 'capabilities', label: t('agentSettings.workbench.capabilityTab') },
    { key: 'providers', label: t('agentSettings.providers.title') },
    { key: 'settings', label: t('agentSettings.workbench.settingsTab') },
  ];
  const dirty = JSON.stringify(document) !== JSON.stringify(original) || displayName !== name;
  const blocked = unavailableCapabilityReferences(document, catalog.capabilities).length > 0;
  useEffect(() => { onDirtyChange?.(dirty); }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  return <main className={styles.editorSurface}>
    <header className={styles.editorHeader}>
      <span className={styles.agentAvatar}><BookmarkOne theme='outline' size={25} /></span>
      <div className={styles.editorHeaderCopy}>
        <div className={styles.headerEyebrow}>{t('agentSettings.workbench.presetBadge')}</div>
        <h2>{name}</h2>
        <p>{t(`agentSettings.template.${path}.description`)}</p>
      </div>
      <Button className={styles.templateReset} size='small' type='text' icon={<Refresh theme='outline' size={14} />} disabled={busy || !dirty} onClick={() => { setDocument(structuredClone(original)); setDisplayName(name); }}>{t('agentSettings.workbench.resetTemplate')}</Button>
    </header>
    <nav className={styles.editorTabs} role='tablist' aria-label={t('agentSettings.title')}>
      {tabs.map((tab) => <button key={tab.key} type='button' role='tab' aria-selected={activeTab === tab.key} aria-controls={`template-panel-${tab.key}`} id={`template-tab-${tab.key}`} className={activeTab === tab.key ? styles.activeTab : ''} onClick={() => setActiveTab(tab.key)}>{tab.label}</button>)}
    </nav>
    <div className={`${styles.editorBody} ${activeTab === 'capabilities' ? styles.capabilityBody : ''}`}>
      {activeTab === 'capabilities' && <div className={styles.capabilityPanel} role='tabpanel' id='template-panel-capabilities' aria-labelledby='template-tab-capabilities'>
        <AgentCapabilityWorkspace document={document} catalog={catalog.capabilities} disabled={busy} onChange={setDocument} />
      </div>}
      {activeTab === 'settings' && <div role='tabpanel' id='template-panel-settings' aria-labelledby='template-tab-settings'>
        <section className={styles.section}>
          <div className={styles.formGrid}>
            <div className={`${styles.field} ${styles.fieldWide}`}>
              <span>{t('agentSettings.runtimeEngine.label')}</span>
              <AgentRuntimeEngineSelector
                value={document.runtime_engine}
                disabled={busy}
                onChange={(runtime_engine) => setDocument((current) => ({ ...current, runtime_engine }))}
              />
            </div>
          </div>
        </section>
      </div>}
      {activeTab === 'providers' && <div role='tabpanel' id='template-panel-providers' aria-labelledby='template-tab-providers'>
        <AgentRoleProviderPicker document={document} catalog={catalog} disabled={busy} onChange={setDocument} />
      </div>}
    </div>
    <footer className={styles.actionBar}>
      <label className={styles.footerName}><span>{t('agentSettings.workbench.customName')}</span><Input value={displayName} maxLength={80} disabled={busy} onChange={setDisplayName} onInput={(event) => setDisplayName((event.target as HTMLInputElement).value)} aria-label={t('agentSettings.workbench.customName')} /></label>
      <div className={styles.templateSaveState}><span className={blocked ? styles.statusWarningDot : styles.statusReadyDot} /><span>{t(blocked ? 'agentSettings.workbench.disabledSave' : 'agentSettings.workbench.readyToSave')}</span></div>
      <Button type='primary' icon={<Save theme='outline' size={16} />} loading={busy} disabled={busy || blocked || !displayName.trim()} onClick={() => onSave(displayName.trim(), document, t(`agentSettings.template.${path}.description`))}>{t('agentSettings.workbench.saveAsMine')}</Button>
    </footer>
  </main>;
};

export default OfficialTemplateOverview;
