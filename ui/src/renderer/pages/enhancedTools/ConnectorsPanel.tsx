/**
 * Connector management surface — third-party account connections live here
 * instead of as standalone product entries. The Feishu connector card hosts
 * the same connection dialog as the Feishu knowledge page, so credentials and
 * permissions stay managed in one place.
 */
import { officeHub } from '@/common/adapter/officeHubBridge';
import { CONNECTORS } from '@/common/connectors';
import FeishuLogo from '@/renderer/assets/channel-logos/lark.svg';
import { ConnectionDialog } from '@/renderer/pages/officeHub/ConnectionDialog';
import { AuthorizationPanel, OfficeBadge, OfficeButton, OfficeDialog } from '@/renderer/pages/officeHub/dialogs';
import { runtimeCanPrepare, runtimeCanRepair } from '@/renderer/pages/officeHub/model';
import { useOfficeConnection } from '@/renderer/pages/officeHub/useOfficeConnection';
import '@/renderer/pages/officeHub/styles.css';
import { DocSearch, Refresh, Right, Setting } from '@icon-park/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { OfficeStatus } from '@/common/types/officeHub';

type ConnectorState = 'preparing' | 'runtime' | 'disabled' | 'not_configured' | 'needs_login' | 'ready' | 'expired' | 'error';

const connectorState = (status: OfficeStatus): ConnectorState => {
  if (status.runtime.phase === 'preparing') return 'preparing';
  if (status.runtime.phase !== 'ready') return 'runtime';
  return status.phase === 'ready' ? 'ready' : status.phase;
};

export default function ConnectorsPanel() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const connection = useOfficeConnection({ onNeedSettings: () => setDialog('connection') });
  const { hub, busy, actionError, setActionError, authVisible, run, startAuth, closeAuth, signedOut, errorPanel, authorization } = connection;
  const [dialog, setDialog] = useState<'connection' | null>(null);
  const feishu = CONNECTORS[0];
  const status = hub.status;
  const state = status ? connectorState(status) : null;
  const connected = state === 'ready';
  return <div className='connectors-panel'>
    <div className='connectors-panel__body' inert={!!dialog || authVisible}>
      {errorPanel(hub.statusError)}{errorPanel(actionError)}
      <article className='connector-card' data-testid='connector-card-feishu'>
        <header className='connector-card__header'>
          <img className='connector-card__logo' src={FeishuLogo} alt='' aria-hidden='true' />
          <div className='connector-card__title'>
            <h3>{t('settings.connectors.feishu.name')}</h3>
            <p className='office-muted office-small'>{t('settings.connectors.feishu.description')}</p>
          </div>
          <OfficeBadge state={connected ? 'good' : state === 'disabled' || state === 'not_configured' ? 'neutral' : 'warning'}>
            {status ? status.runtime.phase !== 'ready' ? t('office.runtime.phase.' + status.runtime.phase) : t('settings.connectors.status.' + state) : t('office.loading')}
          </OfficeBadge>
        </header>
        {status && <dl className='connector-card__meta office-small'>
          <div><dt>{t('office.account.currentAccount')}</dt><dd>{status.actor?.name || '—'}</dd></div>
          <div><dt>{t('settings.connectors.connectionName')}</dt><dd>{status.connection.name || t('settings.connectors.feishu.name')}{status.connection.enabled ? '' : ` · ${t('settings.connectors.status.disabled')}`}</dd></div>
        </dl>}
        <section className='connector-card__capabilities'>
          <h4>{t('settings.connectors.capabilities')}</h4>
          {!connected && <p className='office-muted office-small'>{t('settings.connectors.configureFirst')}</p>}
          <ul>
            <li>
              <button type='button' className='connector-capability' disabled={!connected} onClick={() => navigate(feishu.knowledgePath!)}>
                <DocSearch size={16} />
                <span><strong>{t('knowledge.feishu.title')}</strong><small className='office-muted'>{t('settings.connectors.knowledgeHint')}</small></span>
                <Right size={13} />
              </button>
            </li>
            {feishu.providedSkills.map(skill => <li key={skill}>
              <div className='connector-capability connector-capability--static' title={skill}>
                <Setting size={16} />
                <span><strong>{t('settings.connectors.feishu.skillName')}</strong><small className='office-muted'>{t('settings.connectors.skillHint')}</small></span>
                <code className='connector-capability__name'>{skill}</code>
              </div>
            </li>)}
          </ul>
        </section>
        <footer className='connector-card__actions'>
          {status && runtimeCanPrepare(status.runtime) && <OfficeButton primary disabled={busy} onClick={() => void run(async () => { try { await officeHub.prepare(); } finally { await hub.refreshStatus(); } })}>{t(status.runtime.phase === 'error' ? 'office.runtime.retryPrepare' : 'office.actions.prepare')}</OfficeButton>}
          {status && runtimeCanRepair(status.runtime) && <OfficeButton primary disabled={busy} onClick={() => void run(async () => { try { await officeHub.repair(); } finally { await hub.refreshStatus(); } })}>{t('office.actions.repair')}</OfficeButton>}
          <OfficeButton primary={status?.runtime.phase === 'ready'} onClick={() => { setActionError(null); setDialog('connection'); }}>{t('office.connection.settings')}</OfficeButton>
          <OfficeButton quiet disabled={hub.statusLoading || busy} aria-label={t('office.actions.refresh')} onClick={() => void hub.refreshStatus()}><Refresh size={14} /></OfficeButton>
        </footer>
      </article>
    </div>
    {dialog === 'connection' && status && <ConnectionDialog status={status} authorization={authorization} onStart={startAuth} refresh={hub.refreshStatus} onClose={() => setDialog(null)} onError={setActionError} onSignedOut={() => { signedOut(); }} />}
    {authVisible && <OfficeDialog title={t('office.auth.title')} onClose={closeAuth} busy={busy} footer={<OfficeButton primary disabled={busy} onClick={closeAuth}>{t('office.auth.return')}</OfficeButton>}><AuthorizationPanel {...authorization} /></OfficeDialog>}
  </div>;
}
