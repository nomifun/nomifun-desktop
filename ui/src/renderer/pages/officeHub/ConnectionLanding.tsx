import type { OfficeStatus } from '@/common/types/officeHub';
import { CheckOne, Download, LinkOne, User } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { OfficeButton } from './dialogs';
import { runtimeCanPrepare, runtimeCanRepair } from './model';

export function ConnectionLanding({ status, busy, onPrepare, onRepair, onConnect, onCheck, onSettings }: {
  status: OfficeStatus; busy: boolean; onPrepare: () => void; onRepair: () => void; onConnect: () => void; onCheck: () => void; onSettings: () => void;
}) {
  const { t } = useTranslation();
  const ready = status.runtime.phase === 'ready';
  const installing = status.runtime.phase === 'preparing';
  const missing = runtimeCanPrepare(status.runtime);
  const repairable = runtimeCanRepair(status.runtime);
  const state = installing ? 'installing' : missing ? 'missing' : !ready ? 'repair' : !status.connection.enabled ? 'disabled' : status.phase === 'expired' ? 'expired' : 'login';
  return <section className='office-connect-screen' aria-label={t('office.connection.welcome')}><div className='office-connect-card'>
    <div className='office-connect-symbol'><LinkOne size={26} /></div><h2>{t('office.connectScreen.' + state + 'Title')}</h2><p className='office-connect-description'>{t('office.connectScreen.' + state + 'Hint')}</p>
    <div className='office-connect-prerequisites'><div data-complete={ready}><span>{ready ? <CheckOne size={19} /> : <Download size={19} />}</span><div><strong>{t('office.connectScreen.component')}</strong><p>{t(ready ? 'office.connectScreen.componentReady' : installing ? 'office.account.installingHint' : 'office.connectScreen.componentHint')}</p></div>{ready && <span className='office-prerequisite-state'>{t('office.account.ready')}</span>}</div><div><span><User size={19} /></span><div><strong>{t('office.connectScreen.account')}</strong><p>{t('office.connectScreen.accountHint')}</p></div></div></div>
    <div className='office-connect-actions'><OfficeButton primary disabled={busy || installing} onClick={missing ? onPrepare : repairable ? onRepair : onConnect}>{t(installing ? 'office.connectScreen.installingAction' : missing ? status.runtime.phase === 'error' ? 'office.runtime.retryPrepare' : 'office.actions.prepare' : repairable ? 'office.actions.repair' : !status.connection.enabled ? 'office.connectScreen.enableAction' : 'office.account.login')}</OfficeButton><OfficeButton quiet disabled={busy || installing} onClick={ready ? onSettings : onCheck}>{t(ready ? 'office.connection.settings' : 'office.actions.check')}</OfficeButton></div><p className='office-connect-footnote'>{t('office.connectScreen.privacy')}</p>
  </div></section>;
}
