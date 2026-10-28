import type { FeishuStatus } from '@/common/types/feishuConnector';
import { CheckOne, Download, LinkOne, User } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { FeishuButton } from './dialogs';
import { runtimeCanPrepare, runtimeCanRepair } from './model';

export function ConnectionLanding({ status, busy, onPrepare, onRepair, onConnect, onCheck, onSettings }: {
  status: FeishuStatus; busy: boolean; onPrepare: () => void; onRepair: () => void; onConnect: () => void; onCheck: () => void; onSettings: () => void;
}) {
  const { t } = useTranslation();
  const ready = status.runtime.phase === 'ready';
  const installing = status.runtime.phase === 'preparing';
  const missing = runtimeCanPrepare(status.runtime);
  const repairable = runtimeCanRepair(status.runtime);
  const state = installing ? 'installing' : missing ? 'missing' : !ready ? 'repair' : !status.connection.enabled ? 'disabled' : status.phase === 'expired' ? 'expired' : 'login';
  return <section className='feishu-connect-screen' aria-label={t('feishu.connection.welcome')}><div className='feishu-connect-card'>
    <div className='feishu-connect-symbol'><LinkOne size={26} /></div><h2>{t('feishu.connectScreen.' + state + 'Title')}</h2><p className='feishu-connect-description'>{t('feishu.connectScreen.' + state + 'Hint')}</p>
    <div className='feishu-connect-prerequisites'><div data-complete={ready}><span>{ready ? <CheckOne size={19} /> : <Download size={19} />}</span><div><strong>{t('feishu.connectScreen.component')}</strong><p>{t(ready ? 'feishu.connectScreen.componentReady' : installing ? 'feishu.account.installingHint' : 'feishu.connectScreen.componentHint')}</p></div>{ready && <span className='feishu-prerequisite-state'>{t('feishu.account.ready')}</span>}</div><div><span><User size={19} /></span><div><strong>{t('feishu.connectScreen.account')}</strong><p>{t('feishu.connectScreen.accountHint')}</p></div></div></div>
    <div className='feishu-connect-actions'><FeishuButton primary disabled={busy || installing} onClick={missing ? onPrepare : repairable ? onRepair : onConnect}>{t(installing ? 'feishu.connectScreen.installingAction' : missing ? status.runtime.phase === 'error' ? 'feishu.runtime.retryPrepare' : 'feishu.actions.prepare' : repairable ? 'feishu.actions.repair' : !status.connection.enabled ? 'feishu.connectScreen.enableAction' : 'feishu.account.login')}</FeishuButton><FeishuButton quiet disabled={busy || installing} onClick={ready ? onSettings : onCheck}>{t(ready ? 'feishu.connection.settings' : 'feishu.actions.check')}</FeishuButton></div><p className='feishu-connect-footnote'>{t('feishu.connectScreen.privacy')}</p>
  </div></section>;
}
