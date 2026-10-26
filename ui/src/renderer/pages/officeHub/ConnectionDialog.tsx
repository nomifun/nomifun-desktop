import { officeHub } from '@/common/adapter/officeHubBridge';
import type { AuthStartRequest, OfficeProfile, OfficeStatus, RuntimeUpdateInfo } from '@/common/types/officeHub';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AuthorizationPanel, OfficeBadge, OfficeButton, OfficeDialog, type AuthorizationProps } from './dialogs';
import { officeError, readGroups, scopesFor, terminalAuth } from './model';

export function ConnectionDialog({ status, authorization, onStart, refresh, onClose, onError }: {
  status: OfficeStatus; authorization: AuthorizationProps; onStart: (request: AuthStartRequest) => Promise<void>;
  refresh: () => Promise<OfficeStatus | null>; onClose: () => void; onError: (error: unknown) => void;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState(status.runtime.phase !== 'ready' ? 1 : authorization.flow && !terminalAuth(authorization.flow.phase) ? authorization.flow.kind === 'login' && status.actor ? 3 : 2 : status.actor && status.connection.enabled ? 3 : 2);
  const [selected, setSelected] = useState<string[]>(['documents', 'agenda']);
  const [existing, setExisting] = useState(!!status.connection.profile);
  const [manual, setManual] = useState(false);
  const [profile, setProfile] = useState(status.connection.profile ?? '');
  const [profiles, setProfiles] = useState<OfficeProfile[] | null>(null);
  const [profilesLoading, setProfilesLoading] = useState(false);
  const [profilesError, setProfilesError] = useState<unknown>(null);
  const [profilesRevision, setProfilesRevision] = useState(0);
  const [configuredProfile, setConfiguredProfile] = useState<string | null>(null);
  const [appId, setAppId] = useState('');
  const [secret, setSecret] = useState('');
  const [path, setPath] = useState(status.runtime.executable ?? '');
  const [update, setUpdate] = useState<RuntimeUpdateInfo | null>(null);
  const [working, setWorking] = useState(false);
  const active = !!authorization.flow && !terminalAuth(authorization.flow.phase);
  const canPrepare = status.runtime.phase === 'missing' || (status.runtime.phase === 'error' && status.runtime.executable === null);
  const busy = working || authorization.busy;
  const capabilities = selected.flatMap(key => readGroups[key as keyof typeof readGroups]);
  const scopes = scopesFor(status, capabilities);
  const desiredProfile = existing && !manual ? profile : configuredProfile;
  const identityReady = !!status.actor && status.connection.enabled && !!desiredProfile && desiredProfile === status.connection.profile;
  useEffect(() => {
    if (step !== 2) return;
    const controller = new AbortController();
    setProfilesLoading(true); setProfilesError(null); setProfiles(null);
    void officeHub.profiles(controller.signal).then(value => { if (!controller.signal.aborted) setProfiles(value); })
      .catch(error => { if (!controller.signal.aborted) setProfilesError(error); })
      .finally(() => { if (!controller.signal.aborted) setProfilesLoading(false); });
    return () => controller.abort();
  }, [step, profilesRevision]);
  useEffect(() => {
    if (configuredProfile && status.connection.profile === configuredProfile && authorization.flow?.phase === 'complete' && authorization.flow.kind !== 'login') {
      setExisting(true); setManual(false); setProfile(configuredProfile); setSecret('');
    }
  }, [configuredProfile, status.connection.profile, authorization.flow?.id, authorization.flow?.phase]);
  const work = async (action: () => Promise<unknown>) => {
    if (busy) return false;
    setWorking(true);
    try { await action(); return true; } catch (error) { onError(error); return false; }
    finally { await refresh(); setWorking(false); }
  };
  const connect = async () => {
    if (!profiles) return;
    if (configuredProfile && desiredProfile === configuredProfile && status.connection.profile === configuredProfile) {
      await onStart({ kind: 'login', profile: configuredProfile, scopes: ['offline_access'], app_id: null, app_secret: null });
    } else if (existing && !manual && profile) {
      const ok = await work(() => officeHub.updateConnection({ name: status.connection.name, enabled: true, profile }));
      if (!ok) return;
      await onStart({ kind: 'login', profile, scopes: ['offline_access'], app_id: null, app_secret: null });
    } else {
      let name = 'nomifun-office', suffix = 1;
      while (profiles.some(item => item.name === name)) name = 'nomifun-office-' + suffix++;
      const value = secret; setSecret('');
      setConfiguredProfile(name);
      await onStart({ kind: manual ? 'configure_existing' : 'configure_new', profile: name, scopes: [], app_id: manual ? appId.trim() : null, app_secret: manual ? value : null });
    }
  };
  return <OfficeDialog title={t('office.actions.connect')} busy={busy} onClose={() => { setSecret(''); onClose(); }} className='office-connection-dialog' footer={<>
    <OfficeButton quiet disabled={busy} onClick={() => step === 2 && !active ? setStep(1) : onClose()}>{t(step === 3 ? 'office.setup.skip' : step === 2 && !active ? 'office.actions.back' : 'office.actions.close')}</OfficeButton>
    {step === 1 && <OfficeButton primary disabled={busy || active || status.runtime.phase === 'preparing'} onClick={() => status.runtime.phase === 'ready' ? setStep(2) : canPrepare ? void work(() => officeHub.prepare()) : void work(refresh)}>{t(status.runtime.phase === 'ready' ? 'office.actions.continue' : canPrepare ? status.runtime.phase === 'error' ? 'office.runtime.retryPrepare' : 'office.actions.prepare' : 'office.actions.check')}</OfficeButton>}
    {step === 2 && <OfficeButton primary disabled={busy || active || (!identityReady && (profilesLoading || !profiles)) || (manual && !(configuredProfile && status.connection.profile === configuredProfile) && (!appId.trim() || !secret)) || (existing && !manual && !profile)} onClick={() => identityReady ? setStep(3) : void connect()}>{t(identityReady ? 'office.actions.continue' : 'office.auth.open')}</OfficeButton>}
    {step === 3 && <OfficeButton primary disabled={busy || active || !selected.length} onClick={() => !scopes.length ? onClose() : void onStart({ kind: 'login', scopes, profile: status.connection.profile, app_id: null, app_secret: null })}>{t(!scopes.length ? 'office.setup.finish' : 'office.setup.grant')}</OfficeButton>}
  </>}>
    <ol className='office-stepper'>{['environment', 'connect', 'permissions'].map((key, index) => <li key={key} aria-current={step === index + 1 ? 'step' : undefined} className={step >= index + 1 ? 'active' : ''}><b>{index + 1}</b>{t('office.setup.' + key)}</li>)}</ol>
    {step === 1 ? <><h3>{t('office.setup.environmentTitle')}</h3><p className='office-muted'>{t('office.setup.environmentHint')}</p><div className='office-setting-row'><span>{t('office.host')}</span><span>{status.host_name} · {status.host_os}</span></div><div className='office-setting-row'><span>{t('office.runtime.title')}</span><OfficeBadge state={status.runtime.phase === 'ready' ? 'good' : 'warning'}>{t('office.runtime.phase.' + status.runtime.phase)}</OfficeBadge></div>
      {status.runtime.message && <p className='office-warning'>{status.runtime.message}</p>}
      <details className='office-advanced'><summary>{t('office.runtime.advanced')}</summary><p>{t('office.runtime.notBundled')}</p><p>{t('office.hostHint')}</p>{!canPrepare && status.runtime.phase !== 'ready' && <p>{t('office.runtime.brokenHint')}</p>}<p>{status.runtime.version} · {status.runtime.source && t('office.runtime.source.' + status.runtime.source)}</p>
        <label className='office-label'>{t('office.runtime.selectedPath')}<input className='office-input' value={path} onChange={event => setPath(event.target.value)} disabled={active} /></label><div className='office-actions'><OfficeButton disabled={busy || active || !path.trim()} onClick={() => void work(() => officeHub.selectRuntime(path.trim()))}>{t('office.actions.select')}</OfficeButton>{status.connection.executable && <OfficeButton disabled={busy || active} onClick={() => void work(() => officeHub.selectRuntime(null))}>{t('office.runtime.automatic')}</OfficeButton>}<OfficeButton disabled={busy} onClick={() => void work(refresh)}>{t('office.actions.check')}</OfficeButton>{status.runtime.phase === 'ready' && <OfficeButton disabled={busy} onClick={() => void work(async () => { setUpdate(await officeHub.checkUpdate()); })}>{t('office.runtime.checkUpdate')}</OfficeButton>}</div>
        {update && <p role='status'>{t('office.runtime.versions', { installed: update.installed_version ?? '—', latest: update.latest_version ?? '—', compatible: update.compatible_version })}<br />{t('office.runtime.checkOnly')}{update.message && <><br />{update.message}</>}</p>}
        <p>{t('office.runtime.fullWizardHint')}</p><code className='office-command'>{status.runtime.official_install_command}</code><OfficeButton quiet onClick={() => void work(() => copyText(status.runtime.official_install_command))}>{t('office.actions.copy')}</OfficeButton>
      </details></> : step === 2 ? <><h3>{t('office.setup.connectTitle')}</h3><p className='office-muted'>{t('office.setup.connectHint')}</p><div className='office-setting-row'><strong>{status.connection.name || t('office.title')}</strong><span>{status.actor?.name || t('office.connection.unknownIdentity')}</span></div>
      {authorization.flow && <AuthorizationPanel {...authorization} error={undefined} completionHint={t(authorization.flow.kind === 'login' ? 'office.setup.identityReady' : 'office.setup.identityNext')} />}
      {profilesLoading && <p className='office-small office-muted' role='status'>{t('office.loading')}</p>}{profilesError && <div className='office-error' role='alert'>{officeError(profilesError)}<OfficeButton quiet onClick={() => setProfilesRevision(value => value + 1)}>{t('office.actions.check')}</OfficeButton></div>}
      <details className='office-advanced'><summary>{t('office.connection.existingApp')}</summary><label className='office-check-row'><input type='checkbox' checked={existing} disabled={active} onChange={event => setExisting(event.target.checked)} />{t('office.connection.useExisting')}</label>{existing && <><label className='office-label'>{t('office.connection.profile')}<select className='office-input' value={profile} disabled={active || profilesLoading || !profiles} onChange={event => setProfile(event.target.value)}><option value=''>{t('office.connection.chooseProfile')}</option>{profiles?.map(item => <option key={item.name} value={item.name}>{item.user_name || item.name} · {item.app_id}</option>)}</select></label><label className='office-check-row'><input type='checkbox' checked={manual} disabled={active} onChange={event => setManual(event.target.checked)} />{t('office.connection.manual')}</label>{manual && <><label className='office-label'>{t('office.connection.appId')}<input className='office-input' value={appId} disabled={active} onChange={event => setAppId(event.target.value)} /></label><label className='office-label'>{t('office.connection.appSecret')}<input className='office-input' type='password' autoComplete='new-password' value={secret} disabled={active} onChange={event => setSecret(event.target.value)} /></label><p className='office-small office-muted'>{t('office.connection.secretHint')}</p></>}</>}</details>
    </> : <><h3>{t('office.setup.permissionsTitle')}</h3><p className='office-muted'>{t('office.setup.permissionsHint')}</p>{Object.keys(readGroups).map(key => <label key={key} className='office-permission-option'><input type='checkbox' checked={selected.includes(key)} disabled={active} onChange={event => setSelected(value => event.target.checked ? [...value, key] : value.filter(item => item !== key))} /><span><strong>{t('office.permissions.groups.' + key)}</strong><small>{t('office.permissions.groupHints.' + key)}</small></span></label>)}{authorization.flow?.kind === 'login' && authorization.flow.phase !== 'complete' && <AuthorizationPanel {...authorization} error={undefined} />}
      <details className='office-advanced'><summary>{t('office.connection.advanced')}</summary><p>{status.actor?.name || status.connection.name}<br />{status.actor?.app_id}</p>{status.permissions.map(permission => <p key={permission.capability}>{t('office.permissions.capabilities.' + permission.capability)} · {t(permission.granted ? 'office.permissions.granted' : 'office.permissions.missing')}</p>)}<OfficeButton disabled={busy || active} onClick={() => setStep(1)}>{t('office.actions.check')}</OfficeButton><OfficeButton disabled={busy || active} onClick={() => void work(() => officeHub.updateConnection({ name: status.connection.name, profile: status.connection.profile, enabled: !status.connection.enabled }))}>{t(status.connection.enabled ? 'office.actions.disable' : 'office.actions.enable')}</OfficeButton></details>
    </>}{authorization.error}
  </OfficeDialog>;
}
