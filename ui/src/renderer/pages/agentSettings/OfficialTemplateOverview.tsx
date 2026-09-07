import type { AgentCatalogResponse, AgentPresetDocument, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { createEmptyAgentPresetDocument } from '@/common/types/agentPlatform';
import { Button, Input } from '@arco-design/web-react';
import { BookmarkOne, Refresh, Save } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';
import { unavailableCapabilityReferences } from './capabilityGroups';
import { TEMPLATE_I18N_PATH } from './model';
import styles from './AgentSettingsPage.module.css';

type Props = {
  template: OfficialPresetTemplate;
  busy: boolean;
  catalog: AgentCatalogResponse;
  onSave: (displayName: string, document: AgentPresetDocument, description: string) => void;
  onDirtyChange?: (dirty: boolean) => void;
};

export const documentFromTemplate = (template: OfficialPresetTemplate): AgentPresetDocument => ({
  ...createEmptyAgentPresetDocument(),
  initial_capabilities: template.seed.initial_capabilities.map((capability) => ({ capability, action_allowlist: [] })),
  on_demand_capabilities: template.seed.on_demand_capabilities.map((capability) => ({ capability, action_allowlist: [] })),
  skill_bindings: structuredClone(template.seed.skill_bindings),
});

const OfficialTemplateOverview: React.FC<Props> = ({ template, busy, catalog, onSave, onDirtyChange }) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const original = useMemo(() => documentFromTemplate(template), [template]);
  const [document, setDocument] = useState<AgentPresetDocument>(original);
  const [displayName, setDisplayName] = useState(name);
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
    <div className={styles.editorBody}>
      <AgentCapabilityWorkspace document={document} catalog={catalog.capabilities} disabled={busy} onChange={setDocument} />
    </div>
    <footer className={styles.actionBar}>
      <label className={styles.footerName}><span>{t('agentSettings.workbench.customName')}</span><Input value={displayName} maxLength={80} disabled={busy} onChange={setDisplayName} aria-label={t('agentSettings.workbench.customName')} /></label>
      <div className={styles.templateSaveState}><span className={blocked ? styles.statusWarningDot : styles.statusReadyDot} /><span>{t(blocked ? 'agentSettings.workbench.disabledSave' : 'agentSettings.workbench.readyToSave')}</span></div>
      <Button type='primary' icon={<Save theme='outline' size={16} />} loading={busy} disabled={busy || blocked || !displayName.trim()} onClick={() => onSave(displayName.trim(), document, t(`agentSettings.template.${path}.description`))}>{t('agentSettings.workbench.saveAsMine')}</Button>
    </footer>
  </main>;
};

export default OfficialTemplateOverview;
