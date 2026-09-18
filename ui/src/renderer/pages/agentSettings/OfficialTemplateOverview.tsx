import type { AgentCatalogResponse, AgentPresetDocument, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { createEmptyAgentPresetDocument } from '@/common/types/agentPlatform';
import { Button, Input } from '@arco-design/web-react';
import { Refresh, Save } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import AgentRoleProviderPicker from './AgentRoleProviderPicker';
import { unavailableModuleReferences } from './capabilityGroups';
import { TEMPLATE_I18N_PATH, editingDocument, type TemplateEditingState } from './model';
import styles from './AgentSettingsPage.module.css';
import { AgentEditorActionBar, AgentEditorActionButton } from './AgentEditorActionBar';
import AgentEditorTabs from './AgentEditorTabs';

type Props = {
  template: OfficialPresetTemplate;
  busy: boolean;
  catalog: AgentCatalogResponse;
  onSave: (displayName: string, document: AgentPresetDocument, description: string) => void;
  onDirtyChange?: (dirty: boolean) => void;
  initialEditing?: TemplateEditingState;
  onEditingChange?: (editing: TemplateEditingState) => void;
};

export const documentFromTemplate = (
  template: OfficialPresetTemplate,
  _catalog?: Pick<AgentCatalogResponse, 'modules'>
): AgentPresetDocument => ({
  ...createEmptyAgentPresetDocument(),
  enabled_capabilities: structuredClone(template.seed.enabled_capabilities),

  skill_bindings: structuredClone(template.seed.skill_bindings),
});

const OfficialTemplateOverview: React.FC<Props> = ({ template, busy, catalog, onSave, onDirtyChange, initialEditing, onEditingChange }) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const original = useMemo(() => documentFromTemplate(template, catalog), [template, catalog]);
  const [document, setDocument] = useState<AgentPresetDocument>(() => ({ ...original, ...initialEditing?.document }));
  const [displayName, setDisplayName] = useState(initialEditing?.displayName ?? name);
  const [activeTab, setActiveTab] = useState(
    initialEditing?.activeTab === 'providers' ? 'providers' : 'capabilities'
  );
  useEffect(() => { onEditingChange?.({ displayName, document: editingDocument(document), activeTab }); },
    [displayName, document, activeTab, onEditingChange]);
  const tabs = [
    { key: 'capabilities', label: t('agentSettings.workbench.capabilityTab') },
    { key: 'providers', label: t('agentSettings.providers.title') },
  ];
  const dirty = JSON.stringify(document) !== JSON.stringify(original) || displayName !== name;
  const blocked = unavailableModuleReferences(document, catalog).length > 0;
  useEffect(() => { onDirtyChange?.(dirty); }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  return <main className={styles.editorSurface}>
    <header className={styles.editorHeader}>
      <div className={styles.editorHeaderCopy}>
        <h2>{name}</h2>
        <p>{t(`agentSettings.template.${path}.description`)}</p>
      </div>
      <Button className={styles.templateReset} size='small' type='text' icon={<Refresh theme='outline' size={14} />} disabled={busy || !dirty} onClick={() => { setDocument(structuredClone(original)); setDisplayName(name); }}>{t('agentSettings.workbench.resetTemplate')}</Button>
    </header>
    <AgentEditorTabs tabs={tabs} active={activeTab} onChange={setActiveTab} idPrefix='template' label={t('agentSettings.title')} />
    <div className={`${styles.editorBody} ${activeTab === 'capabilities' ? styles.capabilityBody : ''}`}>
      {activeTab === 'capabilities' && <div className={styles.capabilityPanel} role='tabpanel' id='template-panel-capabilities' aria-labelledby='template-tab-capabilities'>
        <AgentCapabilityWorkspace document={document} catalog={catalog} disabled={busy} onChange={setDocument} />
      </div>}
      {activeTab === 'providers' && <div role='tabpanel' id='template-panel-providers' aria-labelledby='template-tab-providers'>
        <AgentRoleProviderPicker document={document} catalog={catalog} disabled={busy} onChange={setDocument} />
      </div>}
    </div>
    <AgentEditorActionBar>
      <label className={styles.footerName}><span>{t('agentSettings.workbench.customName')}</span><Input value={displayName} maxLength={80} disabled={busy} onChange={setDisplayName} onInput={(event) => setDisplayName((event.target as HTMLInputElement).value)} aria-label={t('agentSettings.workbench.customName')} /></label>
      <div className={styles.templateSaveState}><span className={blocked ? styles.statusWarningDot : styles.statusReadyDot} /><span>{t(blocked ? 'agentSettings.workbench.disabledSave' : 'agentSettings.workbench.readyToSave')}</span></div>
      <AgentEditorActionButton icon={<Save theme='outline' size={15} fill='currentColor' />} loading={busy} disabled={busy || blocked || !displayName.trim()} onClick={() => onSave(displayName.trim(), document, t(`agentSettings.template.${path}.description`))}>{t('agentSettings.workbench.saveAsMine')}</AgentEditorActionButton>
    </AgentEditorActionBar>
  </main>;
};

export default OfficialTemplateOverview;
