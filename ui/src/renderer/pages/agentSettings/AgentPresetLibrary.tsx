import type { AgentPresetLibraryResponse, AgentPresetSummary, OfficialPresetKey, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { Button, Popconfirm } from '@arco-design/web-react';
import { AddOne, ExpandLeft, Code, Customer, Delete, Loading, Magic, MessageOne, Search, User } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TEMPLATE_I18N_PATH, templateModuleCount } from './model';
import styles from './AgentSettingsPage.module.css';
import ContentSider from '@/renderer/components/layout/ContentSider';

type Selection = { kind: 'template'; template: OfficialPresetTemplate } | { kind: 'preset'; preset: AgentPresetSummary } | null;
type Props = {
  width?: number; resizeHandle?: React.ReactNode; onCollapse?: () => void;
  library: AgentPresetLibraryResponse; selection: Selection; busy: boolean; creating: boolean;
  openingPresetId: string | null; deletingPresetId: string | null;
  onSelectTemplate: (template: OfficialPresetTemplate) => void;
  onSelectPreset: (preset: AgentPresetSummary) => void;
  onCreatePreset: (displayName: string) => void;
  onDeletePreset: (preset: AgentPresetSummary) => void | Promise<void>;
};

const TemplateIcon: React.FC<{ templateKey: OfficialPresetKey }> = ({ templateKey }) => {
  const icons = { 'chat.minimal': MessageOne, 'assistant.general': User, 'coding.codex': Code,
    'companion.default': User, 'customer-service.default': Customer, 'creative-studio.default': Magic };
  const Icon = icons[templateKey];
  return <Icon theme='outline' size={18} />;
};

const AgentPresetLibrary: React.FC<Props> = ({
  width = 300, resizeHandle, onCollapse, library, selection, busy, creating, openingPresetId, deletingPresetId,
  onSelectTemplate, onSelectPreset, onCreatePreset, onDeletePreset,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [mode, setMode] = useState<'mine' | 'official'>(selection?.kind === 'preset' ? 'mine' : 'official');
  const tabRefs = useRef(new Map<'mine' | 'official', HTMLButtonElement>());
  useEffect(() => { setMode(selection?.kind === 'preset' ? 'mine' : 'official'); }, [selection?.kind]);
  const matches = (name: string, description = '') => `${name} ${description}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  const templates = library.official_templates.filter((template) => {
    const path = TEMPLATE_I18N_PATH[template.template_key];
    return matches(t(`agentSettings.template.${path}.name`), t(`agentSettings.template.${path}.description`));
  });
  const presets = library.user_presets.filter((preset) => matches(preset.display_name, preset.description));
  const minimalTemplate = library.official_templates.find((template) => template.template_key === 'chat.minimal');
  const activateTab = (next: 'mine' | 'official') => {
    setMode(next);
    requestAnimationFrame(() => tabRefs.current.get(next)?.focus());
  };
  const tabKey = (event: React.KeyboardEvent<HTMLButtonElement>, current: 'mine' | 'official') => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' || (event.key === 'ArrowLeft' && current === 'official')
      ? 'mine'
      : event.key === 'End' || (event.key === 'ArrowRight' && current === 'mine')
        ? 'official'
        : current;
    activateTab(next);
  };

  return <ContentSider width={width} resizeHandle={resizeHandle} className={styles.library} ariaLabel={t('agentSettings.library.ariaLabel')} header={<>
    <div className={styles.libraryHeader}>
      <div className={styles.libraryTitle}>{t('agentSettings.title')}<button type='button' onClick={onCollapse} aria-label={t('agentSettings.workbench.hideList')} title={t('agentSettings.workbench.hideList')}><ExpandLeft theme='outline' size={15} /></button></div>
      <Button size='small' icon={<AddOne theme='outline' size={15} />} loading={creating} disabled={busy} onClick={() => onCreatePreset(t('agentSettings.defaults.untitledName'))}>{t('agentSettings.actions.create')}</Button>
    </div>
    <label className={styles.librarySearch}><Search theme='outline' size={15} /><input type='search' value={query} placeholder={t('agentSettings.workbench.librarySearch')} aria-label={t('agentSettings.workbench.librarySearch')} onChange={(event) => setQuery(event.target.value)} /></label>
    <div className={styles.libraryTabs} role='tablist' aria-label={t('agentSettings.library.title')}>
      <button ref={node => { if (node) tabRefs.current.set('mine', node); else tabRefs.current.delete('mine'); }} id='agent-library-tab-mine' type='button' role='tab' tabIndex={mode === 'mine' ? 0 : -1} aria-controls='agent-library-panel' aria-selected={mode === 'mine'} onKeyDown={event => tabKey(event, 'mine')} onClick={() => setMode('mine')}>{t('agentSettings.workbench.mine')}<span>{library.user_presets.length}</span></button>
      <button ref={node => { if (node) tabRefs.current.set('official', node); else tabRefs.current.delete('official'); }} id='agent-library-tab-official' type='button' role='tab' tabIndex={mode === 'official' ? 0 : -1} aria-controls='agent-library-panel' aria-selected={mode === 'official'} onKeyDown={event => tabKey(event, 'official')} onClick={() => setMode('official')}>{t('agentSettings.workbench.official')}<span>{library.official_templates.length}</span></button>
    </div>
    </>}>
    <div className={styles.libraryBody} id='agent-library-panel' role='tabpanel' aria-labelledby={`agent-library-tab-${mode}`}>
      {library.fresh_start.user_preset_count === 0 && (
        <section className={styles.firstRun} aria-label={t('agentSettings.workbench.firstRunTitle')}>
          <strong>{t('agentSettings.workbench.firstRunTitle')}</strong>
          <p>{t('agentSettings.workbench.firstRunHint')}</p>
          <div>
            <Button size='small' disabled={busy || !minimalTemplate} onClick={() => {
              if (minimalTemplate) onSelectTemplate(minimalTemplate);
            }}>{t('agentSettings.workbench.startMinimal')}</Button>
            <Button size='small' type='text' disabled={busy} onClick={() => onCreatePreset(t('agentSettings.defaults.untitledName'))}>
              {t('agentSettings.workbench.startCustom')}
            </Button>
          </div>
        </section>
      )}
      <p className={styles.libraryHint}>{t(mode === 'mine' ? 'agentSettings.workbench.myHint' : 'agentSettings.workbench.officialHint')}</p>
      {mode === 'official' ? <div className={styles.libraryList}>
        {templates.map((template) => {
          const path = TEMPLATE_I18N_PATH[template.template_key];
          const name = t(`agentSettings.template.${path}.name`);
          const selected = selection?.kind === 'template' && selection.template.template_key === template.template_key;
          return <button type='button' key={template.template_key} className={`${styles.libraryRow} ${selected ? styles.libraryRowActive : ''}`} aria-pressed={selected} disabled={busy} onClick={() => onSelectTemplate(template)}>
            <span className={styles.libraryIcon}><TemplateIcon templateKey={template.template_key} /></span>
            <span className={styles.libraryCopy}><span className={styles.libraryName}>{name}</span><span className={styles.libraryMeta}>{template.template_key === 'assistant.general' && <em>{t('agentSettings.library.recommended')}</em>}{t('agentSettings.library.moduleCount', { count: templateModuleCount(template) })}</span></span>
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
  </ContentSider>;
};

export default AgentPresetLibrary;
