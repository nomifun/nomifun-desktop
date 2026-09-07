import type { AgentPresetLibraryResponse, AgentPresetSummary, OfficialPresetKey, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { Button, Popconfirm } from '@arco-design/web-react';
import { AddOne, Code, Customer, Delete, Loading, Magic, MessageOne, Robot, Search, User } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TEMPLATE_I18N_PATH, templateCapabilityCount } from './model';
import styles from './AgentSettingsPage.module.css';

type Selection = { kind: 'template'; template: OfficialPresetTemplate } | { kind: 'preset'; preset: AgentPresetSummary } | null;
type Props = {
  library: AgentPresetLibraryResponse; selection: Selection; busy: boolean; creating: boolean;
  openingPresetId: string | null; deletingPresetId: string | null;
  onSelectTemplate: (template: OfficialPresetTemplate) => void;
  onSelectPreset: (preset: AgentPresetSummary) => void;
  onCreatePreset: (displayName: string) => void;
  onDeletePreset: (preset: AgentPresetSummary) => void | Promise<void>;
};

const TemplateIcon: React.FC<{ templateKey: OfficialPresetKey }> = ({ templateKey }) => {
  const icons = { 'chat.minimal': MessageOne, 'assistant.general': User, 'coding.codex': Code,
    'companion.default': User, 'robot.default': Robot, 'customer-service.default': Customer, 'creative-studio.default': Magic };
  const Icon = icons[templateKey];
  return <Icon theme='outline' size={18} />;
};

const AgentPresetLibrary: React.FC<Props> = ({
  library, selection, busy, creating, openingPresetId, deletingPresetId,
  onSelectTemplate, onSelectPreset, onCreatePreset, onDeletePreset,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [mode, setMode] = useState<'mine' | 'official'>(selection?.kind === 'preset' ? 'mine' : 'official');
  useEffect(() => { setMode(selection?.kind === 'preset' ? 'mine' : 'official'); }, [selection?.kind]);
  const matches = (name: string, description = '') => `${name} ${description}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  const templates = library.official_templates.filter((template) => {
    const path = TEMPLATE_I18N_PATH[template.template_key];
    return matches(t(`agentSettings.template.${path}.name`), t(`agentSettings.template.${path}.description`));
  });
  const presets = library.user_presets.filter((preset) => matches(preset.display_name, preset.description));

  return <aside className={styles.library} aria-label={t('agentSettings.library.ariaLabel')}>
    <div className={styles.libraryHeader}>
      <div className={styles.libraryTitle}>{t('agentSettings.title')}</div>
      <Button type='primary' size='small' icon={<AddOne theme='outline' size={15} />} loading={creating} disabled={busy} onClick={() => onCreatePreset(t('agentSettings.defaults.untitledName'))}>{t('agentSettings.actions.create')}</Button>
    </div>
    <label className={styles.librarySearch}><Search theme='outline' size={15} /><input type='search' value={query} placeholder={t('agentSettings.workbench.librarySearch')} aria-label={t('agentSettings.workbench.librarySearch')} onChange={(event) => setQuery(event.target.value)} /></label>
    <div className={styles.libraryTabs} role='tablist' aria-label={t('agentSettings.library.title')}>
      <button type='button' role='tab' aria-selected={mode === 'mine'} onClick={() => setMode('mine')}>{t('agentSettings.workbench.mine')}<span>{library.user_presets.length}</span></button>
      <button type='button' role='tab' aria-selected={mode === 'official'} onClick={() => setMode('official')}>{t('agentSettings.workbench.official')}<span>{library.official_templates.length}</span></button>
    </div>
    <div className={styles.libraryBody}>
      <p className={styles.libraryHint}>{t(mode === 'mine' ? 'agentSettings.workbench.myHint' : 'agentSettings.workbench.officialHint')}</p>
      {mode === 'official' ? <div className={styles.libraryList}>
        {templates.map((template) => {
          const path = TEMPLATE_I18N_PATH[template.template_key];
          const name = t(`agentSettings.template.${path}.name`);
          const selected = selection?.kind === 'template' && selection.template.template_key === template.template_key;
          return <button type='button' key={template.template_key} className={`${styles.libraryRow} ${selected ? styles.libraryRowActive : ''}`} aria-pressed={selected} disabled={busy} onClick={() => onSelectTemplate(template)}>
            <span className={styles.libraryIcon}><TemplateIcon templateKey={template.template_key} /></span>
            <span className={styles.libraryCopy}><span className={styles.libraryName}>{name}</span><span className={styles.libraryMeta}>{t('agentSettings.library.capabilityCount', { count: templateCapabilityCount(template) })}</span></span>
          </button>;
        })}
      </div> : <div className={styles.libraryList}>
        {presets.map((preset) => {
          const selected = (selection?.kind === 'preset' && selection.preset.preset_id === preset.preset_id) || openingPresetId === preset.preset_id;
          return <div key={preset.preset_id} className={`${styles.libraryPersonalRow} ${selected ? styles.libraryRowActive : ''}`}>
            <button type='button' className={styles.librarySelect} disabled={busy} aria-pressed={selected} aria-busy={openingPresetId === preset.preset_id} onClick={() => onSelectPreset(preset)}>
              <span className={styles.libraryIcon}>{openingPresetId === preset.preset_id ? <Loading theme='outline' size={18} className='animate-spin' /> : <User theme='outline' size={18} />}</span>
              <span className={styles.libraryCopy}><span className={styles.libraryName}>{preset.display_name}</span><span className={styles.libraryMeta}>{preset.description || t(preset.current_stable_revision ? 'agentSettings.status.saved' : 'agentSettings.status.dirty')}</span></span>
            </button>
            <Popconfirm title={t('agentSettings.library.deleteConfirmTitle', { name: preset.display_name })} content={t('agentSettings.library.deleteConfirmBody')} okText={t('agentSettings.actions.delete')} cancelText={t('common.cancel')} disabled={busy} okButtonProps={{ status: 'danger' }} onOk={() => onDeletePreset(preset)}>
              <Button type='text' status='danger' size='mini' className={styles.rowAction} title={t('agentSettings.actions.delete')} aria-label={t('agentSettings.library.deleteAria', { name: preset.display_name })} icon={<Delete theme='outline' size={14} />} loading={deletingPresetId === preset.preset_id} disabled={busy} />
            </Popconfirm>
          </div>;
        })}
      </div>}
      {(mode === 'mine' ? presets : templates).length === 0 && <div className={styles.libraryEmpty}><User theme='outline' size={26} /><strong>{t(query ? 'agentSettings.workbench.noAgents' : 'agentSettings.library.empty')}</strong>{!query && <Button size='small' type='text' onClick={() => setMode('official')}>{t('agentSettings.workbench.official')}</Button>}</div>}
    </div>
  </aside>;
};

export default AgentPresetLibrary;
