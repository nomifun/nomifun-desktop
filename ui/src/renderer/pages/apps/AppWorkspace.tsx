import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { Alert, Button, Modal, Tag } from '@arco-design/web-react';
import { ArrowLeft, Refresh } from '@icon-park/react';
import type { ReactNode } from 'react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { isDesktopShell } from '@/renderer/utils/platform';
import AppImportDialog from './AppImportDialog';
import AppImportForm from './AppImportForm';
import AppPreview from './AppPreview';
import AppSettingsForm from './AppSettingsForm';
import { appErrorMessage, type AppPreviewFileChange } from './appHostingModel';
import { useAppOperations } from './useAppOperations';
import styles from './AppsPage.module.css';

export interface AppWorkspaceProps {
  app?: HostedApp;
  mode: 'create' | 'import' | 'edit';
  conversation: ReactNode;
  projectHistory?: ReactNode;
  filePanel?: ReactNode;
  developmentBusy?: boolean;
  configurationBusy?: boolean;
  configurationError?: string;
  previewFileChange?: AppPreviewFileChange;
  onFileOperationChange?: (busy: boolean) => void;
  onAppChanged: (app: HostedApp) => void;
  onRefresh: () => void;
  onDeleted: () => void;
}

/** App chrome hosts the ordinary canonical conversation; it does not own a transcript. */
export default function AppWorkspace({ app, mode, conversation, projectHistory, filePanel, developmentBusy = false, configurationBusy = false, configurationError, previewFileChange, onFileOperationChange, onAppChanged, onRefresh, onDeleted }: AppWorkspaceProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [overwrite, setOverwrite] = useState(false);
  const { run, error, setError, busy, pending } = useAppOperations(onAppChanged, onRefresh);
  const working = app ? busy.has(app.id) || Boolean(app.operation) : false;
  const runDisabled = working || developmentBusy || configurationBusy;
  const stopping = app ? pending.current.get(app.id)?.operation === 'stop' || app.operation === 'stopping' : false;
  const hasRunConfiguration = Boolean(app?.settings.preview_command || app?.settings.start_command);
  const remove = () => app && Modal.confirm({ title: t('appHosting.delete.title', { name: app.name }), content: t('appHosting.delete.hint'),
    okText: t('appHosting.actions.delete'), cancelText: t('common.cancel'), okButtonProps: { status: 'danger' },
    onOk: async () => { try { await appHosting.delete.invoke({ id: app.id }); onDeleted(); } catch (e) { setError(appErrorMessage(e)); onRefresh(); throw e; } },
  });
  return <main className={styles.workspace}>
    <header className={styles.workspaceHeader}><Button type='text' icon={<ArrowLeft />} onClick={() => void navigate('/apps')} aria-label={t('appHosting.workspace.back')} />
      <h1>{t('appHosting.title')}<span>/</span>{t(mode === 'import' ? 'appHosting.import.title' : mode === 'create' ? 'appHosting.create.title' : 'appHosting.workspace.editTitle')}</h1>
      {app && <Button type='text' icon={<Refresh />} onClick={onRefresh} aria-label={t('appHosting.actions.refresh')} />}
    </header>
    <div className={styles.workspaceBody}>
      <section className={styles.conversationPane} aria-label={t('appHosting.workspace.conversation')}>{projectHistory}<div className={styles.projectConversation}>{conversation}</div></section>
      <aside className={styles.inspector} aria-label={t('appHosting.workspace.inspector')}>
        {error && <Alert type='error' content={error} closable onClose={() => setError('')} />}
        {configurationError && <Alert type='warning' content={configurationError} />}
        {mode === 'import' && <AppImportForm app={app} developmentBusy={developmentBusy} onFileOperationChange={onFileOperationChange} onImported={onAppChanged} />}
        {!app && mode !== 'import' && <div className={styles.panelEmpty}><h2>{t('appHosting.workspace.inspector')}</h2><p>{t('appHosting.workspace.createHint')}</p></div>}
        {app && <>
          <div className={styles.inspectorControls}>
          <div className={styles.sectionHeading}><h2>{app.name}</h2><Tag color={app.state === 'running' ? 'green' : app.state === 'failed' ? 'red' : 'gray'}>{t(`appHosting.state.${app.state}`)}</Tag></div>
          <div className={styles.actions}>
            {app.state === 'running' ? <>
              {app.preview && <Button type='primary' disabled={runDisabled} onClick={() => void run(app, 'use')}>{t('appHosting.actions.use')}</Button>}
              <Button disabled={stopping} loading={stopping} onClick={() => void run(app, 'stop')}>{t('appHosting.actions.stop')}</Button>
            </> : <>
              {app.operation && <Button disabled={stopping} loading={stopping} onClick={() => void run(app, 'stop')}>{t('appHosting.actions.stop')}</Button>}
              <Button type='primary' disabled={runDisabled} loading={busy.has(app.id) && !stopping} onClick={() => void run(app, app.prepared ? 'start' : 'use')}>{t(app.prepared ? 'appHosting.actions.start' : 'appHosting.actions.use')}</Button>
            </>}
            <Button disabled={runDisabled} onClick={() => void run(app, 'rebuild')}>{t('appHosting.actions.rebuild')}</Button>
          </div>
          {app.description && <p className={styles.inspectorDescription}>{app.description}</p>}
          <p role='status' className={styles.muted}>{t(configurationBusy ? 'appHosting.workspace.configuring'
            : developmentBusy ? 'appHosting.workspace.developing' : !hasRunConfiguration ? 'appHosting.workspace.awaitingConfiguration' : 'appHosting.workspace.configured')}</p>
          </div>
          {app.operation && <p role='status'>{t(`appHosting.operation.${app.operation}`, { defaultValue: app.operation })}</p>}
          {app.last_error && <Alert type='error' content={app.last_error} />}
          <AppSettingsForm key={`${app.id}.settings`} app={app} compact={mode === 'edit' || Boolean(app.development_session_id)} developmentBusy={developmentBusy || configurationBusy} onSaved={onAppChanged} />
          <AppPreview key={`${app.id}.preview`} app={app} previewFileChange={previewFileChange} previewBusy={working} previewDisabled={working || developmentBusy || configurationBusy}
            onPreview={() => run(app, 'preview')} />
          <details className={styles.details}><summary>{t('appHosting.details')}</summary><p className={styles.directory}>{app.directory}</p>
            <p className={styles.muted}>{t('appHosting.workspace.directoryHint')}</p>
            {filePanel && <div className={styles.filePanel}>{filePanel}</div>}
            {isDesktopShell() && <Button size='small' onClick={() => void ipcBridge.shell.openFolderWith.invoke({ folder_path: app.directory, tool: 'explorer' }).catch((e: unknown) => setError(appErrorMessage(e)))}>{t('appHosting.actions.folder')}</Button>}
            {app.output ? <pre>{app.output}</pre> : <p className={styles.muted}>{t('appHosting.workspace.noOutput')}</p>}
          </details>
          {developmentBusy && <p className={styles.muted}>{t('appHosting.workspace.workspaceBusy')}</p>}
          <div className={styles.actions}><Button disabled={runDisabled} onClick={() => setOverwrite(true)}>{t('appHosting.overwrite.action')}</Button><Button status='danger' type='text' disabled={runDisabled} onClick={remove}>{t('appHosting.actions.delete')}</Button></div>
        </>}
      </aside>
    </div>
    {overwrite && app && <AppImportDialog overwrite={app} developmentBusy={developmentBusy} onFileOperationChange={onFileOperationChange} onClose={() => setOverwrite(false)} onImported={onAppChanged} />}
  </main>;
}
