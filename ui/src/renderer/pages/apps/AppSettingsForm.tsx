import { appHosting } from '@/common/adapter/appHostingBridge';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { isConversationAgentTemplate } from '@/renderer/components/agent/conversationAgentCatalog';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import type { AppEnvironmentReport, AppImportKind, AppSettings, HostedApp } from '@/common/types/appHosting';
import { Alert, Button, Input, Radio, Select } from '@arco-design/web-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { openExternalUrl } from '@/renderer/utils/platform';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { appErrorMessage, editableAppSettings, parseAppEnvironment } from './appHostingModel';
import styles from './AppsPage.module.css';

type DraftField = keyof AppSettings | 'name' | 'description' | 'tags' | 'kind';

export default function AppSettingsForm({ app, onSaved, compact = false, developmentBusy = false }: { app: HostedApp; onSaved: (app: HostedApp) => void; compact?: boolean; developmentBusy?: boolean }) {
  const { t } = useTranslation();
  const [name, setName] = useState(app.name);
  const [description, setDescription] = useState(app.description ?? '');
  const [tags, setTags] = useState((app.tags ?? []).join(', '));
  const [kind, setKind] = useState<AppImportKind>(app.import_kind);
  const [settings, setSettings] = useState<AppSettings>(() => editableAppSettings(app.settings));
  const [environment, setEnvironment] = useState(() => Object.entries(app.settings.environment).map(([key, value]) => `${key}=${value}`).join('\n'));
  const [dirtyFields, setDirtyFields] = useState<Set<DraftField>>(() => new Set());
  const dirty = dirtyFields.size > 0;
  const markDirty = (...fields: DraftField[]) => setDirtyFields(current => new Set([...current, ...fields]));
  const [report, setReport] = useState<AppEnvironmentReport>();
  const [busy, setBusy] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState('');
  const { library, presets, error: agentError } = useAgentPresets();
  const agents = new Map<string, string>();
  for (const template of library?.official_templates ?? []) {
    if (!isConversationAgentTemplate(template)) continue;
    const preset = library?.official_presets?.[template.template_key];
    if (preset) agents.set(preset.preset_id, t(`agentSettings.template.${TEMPLATE_I18N_PATH[template.template_key]}.name`));
  }
  for (const preset of presets) agents.set(preset.preset_id, preset.display_name);
  useEffect(() => {
    if (!dirtyFields.has('name')) setName(app.name);
    if (!dirtyFields.has('description')) setDescription(app.description ?? '');
    if (!dirtyFields.has('tags')) setTags((app.tags ?? []).join(', '));
    if (!dirtyFields.has('kind')) setKind(app.import_kind);
    setSettings(current => {
      const updated = editableAppSettings(app.settings);
      for (const key of Object.keys(updated) as (keyof AppSettings)[]) {
        if (dirtyFields.has(key)) Object.assign(updated, { [key]: current[key] });
      }
      return updated;
    });
    if (!dirtyFields.has('environment')) setEnvironment(Object.entries(app.settings.environment).map(([key, value]) => `${key}=${value}`).join('\n'));
  }, [app, dirtyFields]);
  const change = (key: keyof AppSettings, value: string) => { markDirty(key); setSettings((current) => ({ ...current, [key]: value || null })); };
  const fields = ['project_directory', 'install_command', 'preview_command', 'build_command', 'start_command', 'output_directory', 'web_url'] as const;
  const hasForeground = Boolean(settings.preview_command || settings.start_command);
  const fieldPlaceholder = (key: typeof fields[number]) => {
    if (key === 'project_directory') return '.';
    if (key === 'preview_command' && settings.start_command) return t('appHosting.settings.previewFallback');
    if ((key === 'install_command' || key === 'build_command') && hasForeground) return t('appHosting.settings.skippedStep');
    return '';
  };
  const detect = async () => {
    if (busy || developmentBusy || app.operation) return;
    setBusy(true); setDetecting(true); setError('');
    try { onSaved(await appHosting.update.invoke({ id: app.id, detect_settings: true })); }
    catch (e) { setError(appErrorMessage(e)); }
    finally { setBusy(false); setDetecting(false); }
  };
  const save = async () => {
    if (busy) return;
    setBusy(true); setError('');
    try {
      // Sparse edits merge inside the same Host gate as Agent updates. A
      // metadata-only save cannot replace freshly generated runtime settings.
      const patch: Partial<AppSettings> = {};
      for (const key of Object.keys(settings) as (keyof AppSettings)[]) {
        if (dirtyFields.has(key)) Object.assign(patch, { [key]: settings[key] });
      }
      if (dirtyFields.has('project_directory')) patch.project_directory = settings.project_directory?.trim() || '.';
      if (dirtyFields.has('environment')) patch.environment = parseAppEnvironment(environment);
      const value = await appHosting.update.invoke({ id: app.id,
        ...(dirtyFields.has('name') ? { name: name.trim() } : {}),
        ...(dirtyFields.has('description') ? { description: description.trim() } : {}),
        ...(dirtyFields.has('tags') ? { tags: [...new Set(tags.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean))] } : {}),
        ...(dirtyFields.has('kind') ? { import_kind: kind } : {}),
        ...(Object.keys(patch).length ? { settings: patch } : {}) });
      onSaved(value); setDirtyFields(new Set());
    } catch (e) { setError(appErrorMessage(e)); }
    finally { setBusy(false); }
  };
  const metadata = <>
    <label>{t('appHosting.name')}<Input value={name} disabled={busy} onChange={(value) => { markDirty('name'); setName(value); }} aria-label={t('appHosting.name')} /></label>
    <label><span className={styles.fieldHeading}>{t('appHosting.description')}
      {description && <CopyIconButton text={description} size={14} className={styles.copyIcon} />}
    </span><Input.TextArea value={description} disabled={busy} onChange={(value) => { markDirty('description'); setDescription(value); }} rows={2} aria-label={t('appHosting.description')} /></label>
    <label>{t('appHosting.tags')}<Input value={tags} disabled={busy} onChange={(value) => { markDirty('tags'); setTags(value); }} aria-label={t('appHosting.tags')} placeholder={t('appHosting.workspace.tagsHint')} /></label>
  </>;
  return <form className={styles.settings} onSubmit={(event) => { event.preventDefault(); void save(); }}>
    {error && <Alert type='error' content={error} />}
    {compact ? <details className={styles.inspectorCard}><summary>{t('appHosting.actions.edit')}</summary><div className={styles.settings}>{metadata}</div></details>
      : <div className={`${styles.settings} ${styles.inspectorCard}`}>{metadata}</div>}
    <details className={styles.inspectorCard}>
      <summary>{t('appHosting.workspace.advanced')}</summary>
      <div className={styles.settings}>
        <p>{t('appHosting.settings.hint')}</p>
        <Button size='small' loading={detecting} disabled={busy || developmentBusy || Boolean(app.operation)} onClick={() => void detect()}>{t('appHosting.settings.detect')}</Button>
        <p className={styles.muted}>{t('appHosting.settings.detectHint')}</p>
        <label>{t('appHosting.import.kind')}<Radio.Group value={kind} disabled={busy} onChange={(value: AppImportKind) => { markDirty('kind'); setKind(value); }} aria-label={t('appHosting.import.kind')}>
          <Radio value='source'>{t('appHosting.import.source')}</Radio><Radio value='artifact'>{t('appHosting.import.artifact')}</Radio>
        </Radio.Group></label>
        {fields.map((key) => <label key={key}>{t(`appHosting.settings.${key}`)}<Input value={settings[key] ?? ''} disabled={busy} onChange={(value) => change(key, value)} aria-label={t(`appHosting.settings.${key}`)} placeholder={fieldPlaceholder(key)} />
          {key === 'preview_command' && !settings.preview_command && settings.start_command && <p className={styles.muted}>{t('appHosting.settings.effectivePreview', { command: settings.start_command })}</p>}
        </label>)}
        <Button size='small' disabled={busy} onClick={() => {
          markDirty('start_command', 'preview_command', 'output_directory', 'web_url', 'environment'); setSettings((current) => ({ ...current, start_command: 'node .nomifun-static-server.cjs', preview_command: null, output_directory: current.output_directory || '.', web_url: null }));
          setEnvironment((current) => current.split(/\r?\n/).filter((line) => !/^\s*PORT\s*=/.test(line)).join('\n'));
        }}>{t('appHosting.settings.staticPreset')}</Button>
        <p className={styles.muted}>{t('appHosting.settings.staticHint')}</p>
        <label>{t('appHosting.settings.node_path')}<Input value={settings.node_path ?? ''} disabled={busy} onChange={(value) => change('node_path', value)} placeholder={t('appHosting.settings.systemNode')} aria-label={t('appHosting.settings.node_path')} /></label>
        <Button size='small' loading={checking} onClick={() => {
          setChecking(true); setError('');
          void appHosting.environment.invoke({ node_path: settings.node_path }).then(setReport).catch((e: unknown) => setError(appErrorMessage(e))).finally(() => setChecking(false));
        }}>{t('appHosting.settings.checkEnvironment')}</Button>
        {report && <div className={styles.environment}>{(['node', 'npm', 'bun'] as const).map((tool) => <div key={tool}><strong>{tool}</strong><span>{report[tool].version || t('appHosting.settings.missingTool')}</span><small>{report[tool].path || report[tool].error}</small></div>)}
          {!report.node.path && <Button size='small' onClick={() => void openExternalUrl(report.node_download_url)}>{t('appHosting.settings.installNode')}</Button>}
        </div>}
        <label>{t('appHosting.settings.environment')}<Input.TextArea value={environment} disabled={busy} onChange={(value) => { markDirty('environment'); setEnvironment(value); }} rows={4} placeholder={'PORT=4317\nDATA_DIR=./data'} aria-label={t('appHosting.settings.environment')} /></label>
        <p className={styles.muted}>{t('appHosting.settings.environmentHint')}</p>
        <label>{t('appHosting.settings.mcp_url')}<Input value={settings.mcp_url ?? ''} disabled={busy} onChange={(value) => change('mcp_url', value)} aria-label={t('appHosting.settings.mcp_url')} placeholder='http://127.0.0.1:4317/mcp' /></label>
        <div className={styles.agentField}><span>{t('appHosting.settings.agent_id')}</span><Select value={settings.agent_id || undefined} allowClear disabled={busy || app.state === 'running' || Boolean(app.operation)}
          onChange={(value: string | undefined) => change('agent_id', value ?? '')} aria-label={t('appHosting.settings.agent_id')} placeholder={t('appHosting.settings.noAgent')}>
          {settings.agent_id && !agents.has(settings.agent_id) && <Select.Option value={settings.agent_id}>{t('appHosting.settings.unavailableAgent')}</Select.Option>}
          {[...agents].map(([id, label]) => <Select.Option key={id} value={id}>{label}</Select.Option>)}
        </Select></div>
        {agentError && <Alert type='warning' content={appErrorMessage(agentError)} />}
        {app.state === 'running' && <p className={styles.muted}>{t('appHosting.workspace.stopToBindAgent')}</p>}
        <p className={styles.muted}>{t('appHosting.settings.connectionHint')}</p>
      </div>
    </details>
    {dirty && <p className={styles.muted} role='status'>{t('appHosting.settings.unsaved')}</p>}
    {(!compact || dirty || busy) && <Button htmlType='submit' type='primary' loading={busy && !detecting} disabled={busy || !name.trim() || !dirty}>{t('common.save')}</Button>}
  </form>;
}
