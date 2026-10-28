import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type { AuthStartRequest, FeishuProfile, FeishuStatus, RuntimeUpdateInfo } from '@/common/types/feishuConnector';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { CheckOne, Download, SettingConfig, Shield, User } from '@icon-park/react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AuthorizationPanel, FeishuBadge, FeishuButton, FeishuDialog, type AuthorizationProps } from './dialogs';
import { feishuError, readGroups, runtimeCanPrepare, runtimeCanRepair, scopesFor, terminalAuth } from './model';

type ConnectionTab = 'account' | 'access' | 'advanced';
export function ConnectionDialog({ status, authorization, onStart, refresh, onClose, onError, onSignedOut }: {
  status: FeishuStatus; authorization: AuthorizationProps; onStart: (request: AuthStartRequest) => Promise<void>;
  refresh: () => Promise<FeishuStatus | null>; onClose: () => void; onError: (error: unknown) => void; onSignedOut: () => void;
}) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<ConnectionTab>(status.runtime.phase === 'ready' ? 'account' : 'advanced');
  const [profile, setProfile] = useState(status.connection.profile ?? '');
  const [profiles, setProfiles] = useState<FeishuProfile[] | null>(null);
  const [profilesLoading, setProfilesLoading] = useState(false);
  const [profilesError, setProfilesError] = useState<unknown>(null);
  const [profilesRevision, setProfilesRevision] = useState(0);
  const [adding, setAdding] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [manual, setManual] = useState(false);
  const [appId, setAppId] = useState('');
  const [secret, setSecret] = useState('');
  const [path, setPath] = useState(status.runtime.executable ?? '');
  const [update, setUpdate] = useState<RuntimeUpdateInfo | null>(null);
  const [working, setWorking] = useState(false);
  const [confirmation, setConfirmation] = useState<'switch' | 'logout' | null>(null);
  const active = !!authorization.flow && !terminalAuth(authorization.flow.phase);
  const runtimeReady = status.runtime.phase === 'ready';
  const canPrepare = runtimeCanPrepare(status.runtime);
  const canRepair = runtimeCanRepair(status.runtime);
  const connected = runtimeReady && status.connection.enabled && status.phase === 'ready' && !!status.actor;
  const busy = working || authorization.busy;
  const locked = busy || active;
  const previousRuntime = useRef(status.runtime.phase);
  useEffect(() => {
    if (previousRuntime.current === 'preparing' && runtimeReady) setTab('account');
    previousRuntime.current = status.runtime.phase;
  }, [status.runtime.phase, runtimeReady]);
  useEffect(() => {
    if (tab !== 'account' || !runtimeReady || (connected && !choosing && !adding)) return;
    const controller = new AbortController();
    setProfilesLoading(true); setProfilesError(null);
    void feishuConnector.profiles(controller.signal).then(value => {
      if (controller.signal.aborted) return;
      setProfiles(value); setProfile(previous => previous || status.connection.profile || value[0]?.name || '');
    }).catch(error => { if (!controller.signal.aborted) setProfilesError(error); })
      .finally(() => { if (!controller.signal.aborted) setProfilesLoading(false); });
    return () => controller.abort();
  }, [tab, runtimeReady, connected, choosing, adding, profilesRevision]);
  useEffect(() => {
    if (authorization.flow?.phase === 'complete' && authorization.flow.kind !== 'login' && status.connection.profile) {
      setProfile(status.connection.profile); setAdding(false); setManual(false); setSecret(''); setProfilesRevision(value => value + 1);
    }
  }, [authorization.flow?.id, authorization.flow?.phase, status.connection.profile]);
  useEffect(() => {
    if (authorization.flow?.kind === 'login' && ['complete', 'partial'].includes(authorization.flow.phase) && connected) {
      setChoosing(false); setAdding(false);
    }
  }, [authorization.flow?.id, authorization.flow?.phase, connected]);
  const work = async (action: () => Promise<unknown>) => {
    if (busy) return false;
    setWorking(true); onError(null);
    try { await action(); return true; } catch (error) { onError(error); return false; }
    finally { await refresh(); setWorking(false); }
  };
  const login = async () => {
    if (!profiles || locked) return;
    if (!adding && profile) {
      const ok = await work(() => feishuConnector.updateConnection({ name: status.connection.name, enabled: true, profile }));
      if (ok) await onStart({ kind: 'login', profile, scopes: ['offline_access'], app_id: null, app_secret: null });
      return;
    }
    let name = 'nomifun-feishu', suffix = 1;
    while (profiles.some(item => item.name === name)) name = 'nomifun-feishu-' + suffix++;
    const value = secret; setSecret('');
    await onStart({ kind: manual ? 'configure_existing' : 'configure_new', profile: name, scopes: [], app_id: manual ? appId.trim() : null, app_secret: manual ? value : null });
  };
  const signOut = async () => {
    if (!confirmation || !status.connection.profile || locked) return;
    const mode = confirmation, selected = status.connection.profile;
    const ok = await work(async () => {
      await feishuConnector.logout({ profile: selected, expected_actor: status.actor });
      onSignedOut(); setConfirmation(null); setAdding(false); setChoosing(false); setManual(false); setProfile(selected); setProfiles(null);
    });
    if (ok && mode === 'switch') await onStart({ kind: 'login', profile: selected, scopes: ['offline_access'], app_id: null, app_secret: null });
  };
  const prepare = () => work(async () => {
    await feishuConnector.prepare();
    const next = await refresh();
    if (next?.runtime.phase === 'ready') setTab('account');
  });
  const repair = () => work(async () => {
    await feishuConnector.repair();
    const next = await refresh();
    if (next?.runtime.phase === 'ready') setTab('account');
  });
  const showFlow = !!authorization.flow && authorization.flow.phase !== 'complete';
  const icons = { account: User, access: Shield, advanced: SettingConfig };
  return <FeishuDialog title={t('feishu.connection.manage')} busy={busy} onClose={() => { setSecret(''); onClose(); }} className='feishu-connection-dialog' footer={<>
    <span className='feishu-small feishu-muted'>{t('feishu.account.privateHint')}</span><FeishuButton onClick={onClose} disabled={busy}>{t('feishu.account.done')}</FeishuButton>
  </>}>
    <div className='feishu-connection-layout'>
      <nav className='feishu-settings-nav' aria-label={t('feishu.connection.settings')}>
        {(['account', 'access', 'advanced'] as const).map(value => { const Icon = icons[value]; return <button type='button' key={value} aria-current={tab === value ? 'page' : undefined} onClick={() => { setTab(value); setConfirmation(null); }}><Icon size={17} /><span>{t('feishu.account.sections.' + value)}</span></button>; })}
      </nav>
      <div className='feishu-settings-content'>
        {tab === 'account' && <section aria-label={t('feishu.account.sections.account')}>
          <div className='feishu-section-heading'><h3>{t(connected && !choosing && !adding ? 'feishu.account.sections.account' : status.phase === 'expired' ? 'feishu.account.expiredTitle' : !status.connection.enabled ? 'feishu.account.disabledTitle' : adding ? 'feishu.account.addTitle' : 'feishu.account.loginTitle')}</h3><p>{t(connected && !choosing && !adding ? 'feishu.account.accountHint' : !status.connection.enabled ? 'feishu.account.disabledHint' : 'feishu.account.loginHint')}</p></div>
          {!runtimeReady ? <div className='feishu-settings-notice'><Download size={24} /><p>{t('feishu.account.installFirst')}</p><FeishuButton primary onClick={() => setTab('advanced')}>{t('feishu.account.viewComponent')}</FeishuButton></div> : confirmation ? <div className='feishu-signout-confirm' role='alert'>
            <h3>{t(confirmation === 'switch' ? 'feishu.account.switchTitle' : 'feishu.account.logoutTitle')}</h3><p>{t('feishu.account.logoutHint', { name: status.actor?.name || t('feishu.account.currentAccount') })}</p><p className='feishu-small feishu-muted'>{t(confirmation === 'switch' ? 'feishu.account.switchHint' : 'feishu.account.signedOutHint')}</p>
            <div className='feishu-actions'><FeishuButton disabled={locked} onClick={() => setConfirmation(null)}>{t('feishu.actions.cancel')}</FeishuButton><FeishuButton className='feishu-button--danger' disabled={locked} onClick={() => void signOut()}>{t(confirmation === 'switch' ? 'feishu.account.confirmSwitch' : 'feishu.account.confirmLogout')}</FeishuButton></div>
          </div> : <>
            {connected && !choosing && !adding ? <>
              <div className='feishu-account-card'><div className='feishu-account-avatar'><User size={25} /></div><div><strong>{status.actor!.name || status.actor!.user_id}</strong><div className='feishu-account-status'><span className='feishu-status-dot' />{t('feishu.account.connected')}</div></div></div>
              <div className='feishu-account-controls'><FeishuButton disabled={locked} onClick={() => setConfirmation('switch')}>{t('feishu.account.switch')}</FeishuButton><FeishuButton quiet className='feishu-text-danger' disabled={locked} onClick={() => setConfirmation('logout')}>{t('feishu.account.logout')}</FeishuButton></div><p className='feishu-small feishu-muted'>{t('feishu.account.switchHint')}</p>
              <FeishuButton quiet disabled={locked} onClick={() => setChoosing(true)}>{t('feishu.account.otherConnection')}</FeishuButton>
            </> : <>
              {profilesLoading && <p role='status' className='feishu-small feishu-muted'>{t('feishu.account.loadingApps')}</p>}{profilesError && <div className='feishu-error' role='alert'>{feishuError(profilesError)}<FeishuButton quiet onClick={() => setProfilesRevision(value => value + 1)}>{t('feishu.actions.check')}</FeishuButton></div>}
              {!adding && (profiles?.length || profile) ? <label className='feishu-label'>{t('feishu.account.application')}<select className='feishu-input' value={profile} disabled={locked || profilesLoading || !profiles} onChange={event => setProfile(event.target.value)}>{profile && !profiles?.some(item => item.name === profile) && <option value={profile}>{profile}</option>}{profiles?.map(item => <option key={item.name} value={item.name}>{item.name} · {item.app_id}</option>)}</select></label> : <p className='feishu-small feishu-muted'>{t('feishu.account.newAppHint')}</p>}
              {(adding || (!profilesLoading && profiles?.length === 0)) && <details className='feishu-manual-application'><summary>{t('feishu.account.manualApp')}</summary><label className='feishu-check-row'><input type='checkbox' checked={manual} disabled={locked} onChange={event => setManual(event.target.checked)} />{t('feishu.connection.manual')}</label>{manual && <><label className='feishu-label'>{t('feishu.connection.appId')}<input className='feishu-input' value={appId} disabled={locked} onChange={event => setAppId(event.target.value)} /></label><label className='feishu-label'>{t('feishu.connection.appSecret')}<input className='feishu-input' type='password' autoComplete='new-password' value={secret} disabled={locked} onChange={event => setSecret(event.target.value)} /></label><p className='feishu-small feishu-muted'>{t('feishu.connection.secretHint')}</p></>}</details>}
              <div className='feishu-actions feishu-login-actions'><FeishuButton primary disabled={locked || profilesLoading || !profiles || (manual && (!appId.trim() || !secret))} onClick={() => void login()}>{t(!adding && profile ? 'feishu.account.login' : 'feishu.account.connectApp')}</FeishuButton>{!adding && !!profiles?.length && <FeishuButton quiet disabled={locked} onClick={() => setAdding(true)}>{t('feishu.account.otherConnection')}</FeishuButton>}{adding && <FeishuButton quiet disabled={locked} onClick={() => { setAdding(false); setManual(false); setSecret(''); }}>{t('feishu.actions.back')}</FeishuButton>}</div>
              {status.connection.profile && (status.actor || status.phase === 'expired') && <div className='feishu-account-controls'><FeishuButton quiet disabled={locked} onClick={() => setConfirmation('switch')}>{t('feishu.account.switch')}</FeishuButton><FeishuButton quiet className='feishu-text-danger' disabled={locked} onClick={() => setConfirmation('logout')}>{t('feishu.account.logout')}</FeishuButton></div>}
            </>}
          </>}
        </section>}
        {tab === 'access' && <section aria-label={t('feishu.account.sections.access')}>
          <div className='feishu-section-heading'><h3>{t('feishu.account.sections.access')}</h3><p>{t('feishu.account.accessHint')}</p></div>
          {!connected ? <div className='feishu-settings-notice'><Shield size={24} /><p>{t('feishu.account.loginFirst')}</p><FeishuButton onClick={() => setTab('account')}>{t('feishu.account.sections.account')}</FeishuButton></div> : Object.keys(readGroups).map(key => {
            const scopes = scopesFor(status, readGroups[key as keyof typeof readGroups]);
            return <div className='feishu-access-row' key={key}><div><strong>{t('feishu.permissions.groups.' + key)}</strong><p>{t('feishu.permissions.groupHints.' + key)}</p><FeishuBadge state={scopes.length ? 'neutral' : 'good'}>{t(scopes.length ? 'feishu.permissions.missing' : 'feishu.permissions.granted')}</FeishuBadge></div>{scopes.length ? <FeishuButton disabled={locked} onClick={() => void onStart({ kind: 'login', scopes, profile: status.connection.profile, app_id: null, app_secret: null })}>{t('feishu.actions.authorizeRead')}</FeishuButton> : <CheckOne size={18} className='feishu-access-check' />}</div>;
          })}<p className='feishu-small feishu-muted feishu-access-note'>{t('feishu.account.writeAccessHint')}</p>
        </section>}
        {tab === 'advanced' && <section aria-label={t('feishu.account.sections.advanced')}>
          <div className='feishu-section-heading'><h3>{t('feishu.account.componentTitle')}</h3><p>{t('feishu.account.componentHint')}</p></div>
          <div className='feishu-component-summary'><span className='feishu-component-icon'><Download size={20} /></span><div><strong>{t('feishu.runtime.title')}</strong><p>{status.runtime.version ? 'v' + status.runtime.version : t('feishu.runtime.phase.' + status.runtime.phase)}</p></div><FeishuBadge state={runtimeReady ? 'good' : 'warning'}>{t('feishu.runtime.phase.' + status.runtime.phase)}</FeishuBadge></div>
          {status.runtime.message && <p className='feishu-warning'>{status.runtime.message}</p>}{!runtimeReady && !canPrepare && status.runtime.phase !== 'preparing' && <p className='feishu-small feishu-muted'>{t('feishu.runtime.brokenHint')}</p>}
          <div className='feishu-actions'>{canPrepare && <FeishuButton primary disabled={locked} onClick={() => void prepare()}>{t(status.runtime.phase === 'error' ? 'feishu.runtime.retryPrepare' : 'feishu.actions.prepare')}</FeishuButton>}{canRepair && <FeishuButton primary disabled={locked} onClick={() => void repair()}>{t('feishu.actions.repair')}</FeishuButton>}<FeishuButton disabled={busy || status.runtime.phase === 'preparing'} onClick={() => void work(refresh)}>{t('feishu.actions.check')}</FeishuButton>{runtimeReady && <FeishuButton disabled={busy} onClick={() => void work(async () => { setUpdate(await feishuConnector.checkUpdate()); })}>{t('feishu.runtime.checkUpdate')}</FeishuButton>}</div>
          {status.runtime.phase === 'preparing' && <p role='status'>{t('feishu.account.installingHint')}</p>}{update && <p role='status' className='feishu-small feishu-muted'>{t('feishu.runtime.versions', { installed: update.installed_version ?? '—', latest: update.latest_version ?? '—', compatible: update.compatible_version })}<br />{t('feishu.runtime.checkOnly')}{update.message && <><br />{update.message}</>}</p>}
          <details className='feishu-advanced'><summary>{t('feishu.account.technicalDetails')}</summary><dl className='feishu-connection-facts'><dt>{t('feishu.host')}</dt><dd>{status.host_name} · {status.host_os}</dd>{status.runtime.source && <><dt>{t('feishu.account.installationSource')}</dt><dd>{t('feishu.runtime.source.' + status.runtime.source)}</dd></>}{status.connection.profile && <><dt>{t('feishu.connection.profile')}</dt><dd>{status.connection.profile}</dd></>}{status.actor && <><dt>{t('feishu.connection.appId')}</dt><dd>{status.actor.app_id}</dd></>}</dl><label className='feishu-label'>{t('feishu.runtime.selectedPath')}<input className='feishu-input' value={path} onChange={event => setPath(event.target.value)} disabled={locked} /></label><div className='feishu-actions'><FeishuButton disabled={locked || !path.trim()} onClick={() => void work(() => feishuConnector.selectRuntime(path.trim()))}>{t('feishu.actions.select')}</FeishuButton>{status.connection.executable && <FeishuButton disabled={locked} onClick={() => void work(() => feishuConnector.selectRuntime(null))}>{t('feishu.runtime.automatic')}</FeishuButton>}</div><p className='feishu-small feishu-muted'>{t('feishu.hostHint')}</p><p className='feishu-small feishu-muted'>{t('feishu.runtime.fullWizardHint')}</p><code className='feishu-command'>{status.runtime.official_install_command}</code><FeishuButton quiet onClick={() => void work(() => copyText(status.runtime.official_install_command))}>{t('feishu.actions.copy')}</FeishuButton></details>
          <div className='feishu-connection-toggle'><div><strong>{t('feishu.account.connectionUse')}</strong><p>{t('feishu.account.disableHint')}</p></div><FeishuButton disabled={locked} onClick={() => void work(() => feishuConnector.updateConnection({ name: status.connection.name, profile: status.connection.profile, enabled: !status.connection.enabled }))}>{t(status.connection.enabled ? 'feishu.actions.disable' : 'feishu.actions.enable')}</FeishuButton></div>
        </section>}
        {showFlow && <div className='feishu-connection-flow'><AuthorizationPanel {...authorization} error={undefined} completionHint={t(authorization.flow?.phase === 'partial' ? 'feishu.account.partialHint' : 'feishu.account.connected')} /></div>}{authorization.error}
      </div>
    </div>
  </FeishuDialog>;
}
