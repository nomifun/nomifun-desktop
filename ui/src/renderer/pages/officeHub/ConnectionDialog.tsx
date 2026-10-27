import { officeHub } from '@/common/adapter/officeHubBridge';
import type { AuthStartRequest, OfficeProfile, OfficeStatus, RuntimeUpdateInfo } from '@/common/types/officeHub';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { CheckOne, Download, SettingConfig, Shield, User } from '@icon-park/react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AuthorizationPanel, OfficeBadge, OfficeButton, OfficeDialog, type AuthorizationProps } from './dialogs';
import { officeError, readGroups, runtimeCanPrepare, runtimeCanRepair, scopesFor, terminalAuth } from './model';

type ConnectionTab = 'account' | 'access' | 'advanced';
export function ConnectionDialog({ status, authorization, onStart, refresh, onClose, onError, onSignedOut }: {
  status: OfficeStatus; authorization: AuthorizationProps; onStart: (request: AuthStartRequest) => Promise<void>;
  refresh: () => Promise<OfficeStatus | null>; onClose: () => void; onError: (error: unknown) => void; onSignedOut: () => void;
}) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<ConnectionTab>(status.runtime.phase === 'ready' ? 'account' : 'advanced');
  const [profile, setProfile] = useState(status.connection.profile ?? '');
  const [profiles, setProfiles] = useState<OfficeProfile[] | null>(null);
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
    void officeHub.profiles(controller.signal).then(value => {
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
      const ok = await work(() => officeHub.updateConnection({ name: status.connection.name, enabled: true, profile }));
      if (ok) await onStart({ kind: 'login', profile, scopes: ['offline_access'], app_id: null, app_secret: null });
      return;
    }
    let name = 'nomifun-office', suffix = 1;
    while (profiles.some(item => item.name === name)) name = 'nomifun-office-' + suffix++;
    const value = secret; setSecret('');
    await onStart({ kind: manual ? 'configure_existing' : 'configure_new', profile: name, scopes: [], app_id: manual ? appId.trim() : null, app_secret: manual ? value : null });
  };
  const signOut = async () => {
    if (!confirmation || !status.connection.profile || locked) return;
    const mode = confirmation, selected = status.connection.profile;
    const ok = await work(async () => {
      await officeHub.logout({ profile: selected, expected_actor: status.actor });
      onSignedOut(); setConfirmation(null); setAdding(false); setChoosing(false); setManual(false); setProfile(selected); setProfiles(null);
    });
    if (ok && mode === 'switch') await onStart({ kind: 'login', profile: selected, scopes: ['offline_access'], app_id: null, app_secret: null });
  };
  const prepare = () => work(async () => {
    await officeHub.prepare();
    const next = await refresh();
    if (next?.runtime.phase === 'ready') setTab('account');
  });
  const repair = () => work(async () => {
    await officeHub.repair();
    const next = await refresh();
    if (next?.runtime.phase === 'ready') setTab('account');
  });
  const showFlow = !!authorization.flow && authorization.flow.phase !== 'complete';
  const icons = { account: User, access: Shield, advanced: SettingConfig };
  return <OfficeDialog title={t('office.connection.manage')} busy={busy} onClose={() => { setSecret(''); onClose(); }} className='office-connection-dialog' footer={<>
    <span className='office-small office-muted'>{t('office.account.privateHint')}</span><OfficeButton onClick={onClose} disabled={busy}>{t('office.account.done')}</OfficeButton>
  </>}>
    <div className='office-connection-layout'>
      <nav className='office-settings-nav' aria-label={t('office.connection.settings')}>
        {(['account', 'access', 'advanced'] as const).map(value => { const Icon = icons[value]; return <button type='button' key={value} aria-current={tab === value ? 'page' : undefined} onClick={() => { setTab(value); setConfirmation(null); }}><Icon size={17} /><span>{t('office.account.sections.' + value)}</span></button>; })}
      </nav>
      <div className='office-settings-content'>
        {tab === 'account' && <section aria-label={t('office.account.sections.account')}>
          <div className='office-section-heading'><h3>{t(connected && !choosing && !adding ? 'office.account.sections.account' : status.phase === 'expired' ? 'office.account.expiredTitle' : !status.connection.enabled ? 'office.account.disabledTitle' : adding ? 'office.account.addTitle' : 'office.account.loginTitle')}</h3><p>{t(connected && !choosing && !adding ? 'office.account.accountHint' : !status.connection.enabled ? 'office.account.disabledHint' : 'office.account.loginHint')}</p></div>
          {!runtimeReady ? <div className='office-settings-notice'><Download size={24} /><p>{t('office.account.installFirst')}</p><OfficeButton primary onClick={() => setTab('advanced')}>{t('office.account.viewComponent')}</OfficeButton></div> : confirmation ? <div className='office-signout-confirm' role='alert'>
            <h3>{t(confirmation === 'switch' ? 'office.account.switchTitle' : 'office.account.logoutTitle')}</h3><p>{t('office.account.logoutHint', { name: status.actor?.name || t('office.account.currentAccount') })}</p><p className='office-small office-muted'>{t(confirmation === 'switch' ? 'office.account.switchHint' : 'office.account.signedOutHint')}</p>
            <div className='office-actions'><OfficeButton disabled={locked} onClick={() => setConfirmation(null)}>{t('office.actions.cancel')}</OfficeButton><OfficeButton className='office-button--danger' disabled={locked} onClick={() => void signOut()}>{t(confirmation === 'switch' ? 'office.account.confirmSwitch' : 'office.account.confirmLogout')}</OfficeButton></div>
          </div> : <>
            {connected && !choosing && !adding ? <>
              <div className='office-account-card'><div className='office-account-avatar'><User size={25} /></div><div><strong>{status.actor!.name || status.actor!.user_id}</strong><div className='office-account-status'><span className='office-status-dot' />{t('office.account.connected')}</div></div></div>
              <div className='office-account-controls'><OfficeButton disabled={locked} onClick={() => setConfirmation('switch')}>{t('office.account.switch')}</OfficeButton><OfficeButton quiet className='office-text-danger' disabled={locked} onClick={() => setConfirmation('logout')}>{t('office.account.logout')}</OfficeButton></div><p className='office-small office-muted'>{t('office.account.switchHint')}</p>
              <OfficeButton quiet disabled={locked} onClick={() => setChoosing(true)}>{t('office.account.otherConnection')}</OfficeButton>
            </> : <>
              {profilesLoading && <p role='status' className='office-small office-muted'>{t('office.account.loadingApps')}</p>}{profilesError && <div className='office-error' role='alert'>{officeError(profilesError)}<OfficeButton quiet onClick={() => setProfilesRevision(value => value + 1)}>{t('office.actions.check')}</OfficeButton></div>}
              {!adding && (profiles?.length || profile) ? <label className='office-label'>{t('office.account.application')}<select className='office-input' value={profile} disabled={locked || profilesLoading || !profiles} onChange={event => setProfile(event.target.value)}>{profile && !profiles?.some(item => item.name === profile) && <option value={profile}>{profile}</option>}{profiles?.map(item => <option key={item.name} value={item.name}>{item.name} · {item.app_id}</option>)}</select></label> : <p className='office-small office-muted'>{t('office.account.newAppHint')}</p>}
              {(adding || (!profilesLoading && profiles?.length === 0)) && <details className='office-manual-application'><summary>{t('office.account.manualApp')}</summary><label className='office-check-row'><input type='checkbox' checked={manual} disabled={locked} onChange={event => setManual(event.target.checked)} />{t('office.connection.manual')}</label>{manual && <><label className='office-label'>{t('office.connection.appId')}<input className='office-input' value={appId} disabled={locked} onChange={event => setAppId(event.target.value)} /></label><label className='office-label'>{t('office.connection.appSecret')}<input className='office-input' type='password' autoComplete='new-password' value={secret} disabled={locked} onChange={event => setSecret(event.target.value)} /></label><p className='office-small office-muted'>{t('office.connection.secretHint')}</p></>}</details>}
              <div className='office-actions office-login-actions'><OfficeButton primary disabled={locked || profilesLoading || !profiles || (manual && (!appId.trim() || !secret))} onClick={() => void login()}>{t(!adding && profile ? 'office.account.login' : 'office.account.connectApp')}</OfficeButton>{!adding && !!profiles?.length && <OfficeButton quiet disabled={locked} onClick={() => setAdding(true)}>{t('office.account.otherConnection')}</OfficeButton>}{adding && <OfficeButton quiet disabled={locked} onClick={() => { setAdding(false); setManual(false); setSecret(''); }}>{t('office.actions.back')}</OfficeButton>}</div>
              {status.connection.profile && (status.actor || status.phase === 'expired') && <div className='office-account-controls'><OfficeButton quiet disabled={locked} onClick={() => setConfirmation('switch')}>{t('office.account.switch')}</OfficeButton><OfficeButton quiet className='office-text-danger' disabled={locked} onClick={() => setConfirmation('logout')}>{t('office.account.logout')}</OfficeButton></div>}
            </>}
          </>}
        </section>}
        {tab === 'access' && <section aria-label={t('office.account.sections.access')}>
          <div className='office-section-heading'><h3>{t('office.account.sections.access')}</h3><p>{t('office.account.accessHint')}</p></div>
          {!connected ? <div className='office-settings-notice'><Shield size={24} /><p>{t('office.account.loginFirst')}</p><OfficeButton onClick={() => setTab('account')}>{t('office.account.sections.account')}</OfficeButton></div> : Object.keys(readGroups).map(key => {
            const scopes = scopesFor(status, readGroups[key as keyof typeof readGroups]);
            return <div className='office-access-row' key={key}><div><strong>{t('office.permissions.groups.' + key)}</strong><p>{t('office.permissions.groupHints.' + key)}</p><OfficeBadge state={scopes.length ? 'neutral' : 'good'}>{t(scopes.length ? 'office.permissions.missing' : 'office.permissions.granted')}</OfficeBadge></div>{scopes.length ? <OfficeButton disabled={locked} onClick={() => void onStart({ kind: 'login', scopes, profile: status.connection.profile, app_id: null, app_secret: null })}>{t('office.actions.authorizeRead')}</OfficeButton> : <CheckOne size={18} className='office-access-check' />}</div>;
          })}<p className='office-small office-muted office-access-note'>{t('office.account.writeAccessHint')}</p>
        </section>}
        {tab === 'advanced' && <section aria-label={t('office.account.sections.advanced')}>
          <div className='office-section-heading'><h3>{t('office.account.componentTitle')}</h3><p>{t('office.account.componentHint')}</p></div>
          <div className='office-component-summary'><span className='office-component-icon'><Download size={20} /></span><div><strong>{t('office.runtime.title')}</strong><p>{status.runtime.version ? 'v' + status.runtime.version : t('office.runtime.phase.' + status.runtime.phase)}</p></div><OfficeBadge state={runtimeReady ? 'good' : 'warning'}>{t('office.runtime.phase.' + status.runtime.phase)}</OfficeBadge></div>
          {status.runtime.message && <p className='office-warning'>{status.runtime.message}</p>}{!runtimeReady && !canPrepare && status.runtime.phase !== 'preparing' && <p className='office-small office-muted'>{t('office.runtime.brokenHint')}</p>}
          <div className='office-actions'>{canPrepare && <OfficeButton primary disabled={locked} onClick={() => void prepare()}>{t(status.runtime.phase === 'error' ? 'office.runtime.retryPrepare' : 'office.actions.prepare')}</OfficeButton>}{canRepair && <OfficeButton primary disabled={locked} onClick={() => void repair()}>{t('office.actions.repair')}</OfficeButton>}<OfficeButton disabled={busy || status.runtime.phase === 'preparing'} onClick={() => void work(refresh)}>{t('office.actions.check')}</OfficeButton>{runtimeReady && <OfficeButton disabled={busy} onClick={() => void work(async () => { setUpdate(await officeHub.checkUpdate()); })}>{t('office.runtime.checkUpdate')}</OfficeButton>}</div>
          {status.runtime.phase === 'preparing' && <p role='status'>{t('office.account.installingHint')}</p>}{update && <p role='status' className='office-small office-muted'>{t('office.runtime.versions', { installed: update.installed_version ?? '—', latest: update.latest_version ?? '—', compatible: update.compatible_version })}<br />{t('office.runtime.checkOnly')}{update.message && <><br />{update.message}</>}</p>}
          <details className='office-advanced'><summary>{t('office.account.technicalDetails')}</summary><dl className='office-connection-facts'><dt>{t('office.host')}</dt><dd>{status.host_name} · {status.host_os}</dd>{status.runtime.source && <><dt>{t('office.account.installationSource')}</dt><dd>{t('office.runtime.source.' + status.runtime.source)}</dd></>}{status.connection.profile && <><dt>{t('office.connection.profile')}</dt><dd>{status.connection.profile}</dd></>}{status.actor && <><dt>{t('office.connection.appId')}</dt><dd>{status.actor.app_id}</dd></>}</dl><label className='office-label'>{t('office.runtime.selectedPath')}<input className='office-input' value={path} onChange={event => setPath(event.target.value)} disabled={locked} /></label><div className='office-actions'><OfficeButton disabled={locked || !path.trim()} onClick={() => void work(() => officeHub.selectRuntime(path.trim()))}>{t('office.actions.select')}</OfficeButton>{status.connection.executable && <OfficeButton disabled={locked} onClick={() => void work(() => officeHub.selectRuntime(null))}>{t('office.runtime.automatic')}</OfficeButton>}</div><p className='office-small office-muted'>{t('office.hostHint')}</p><p className='office-small office-muted'>{t('office.runtime.fullWizardHint')}</p><code className='office-command'>{status.runtime.official_install_command}</code><OfficeButton quiet onClick={() => void work(() => copyText(status.runtime.official_install_command))}>{t('office.actions.copy')}</OfficeButton></details>
          <div className='office-connection-toggle'><div><strong>{t('office.account.connectionUse')}</strong><p>{t('office.account.disableHint')}</p></div><OfficeButton disabled={locked} onClick={() => void work(() => officeHub.updateConnection({ name: status.connection.name, profile: status.connection.profile, enabled: !status.connection.enabled }))}>{t(status.connection.enabled ? 'office.actions.disable' : 'office.actions.enable')}</OfficeButton></div>
        </section>}
        {showFlow && <div className='office-connection-flow'><AuthorizationPanel {...authorization} error={undefined} completionHint={t(authorization.flow?.phase === 'partial' ? 'office.account.partialHint' : 'office.account.connected')} /></div>}{authorization.error}
      </div>
    </div>
  </OfficeDialog>;
}
