import type { AuthorizationFlow, OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { Close, LinkOne } from '@icon-park/react';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { errorDetails, mutationTitle, officeError, operationKind, pendingItem, relatedAuthorization, relatedOperation, sameActor, terminalAuth } from './model';

export function OfficeButton({ children, primary, quiet, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean; quiet?: boolean }) {
  return <button type='button' {...props} className={['office-button', primary ? 'office-button--primary' : '', quiet ? 'office-button--quiet' : '', props.className ?? ''].join(' ')}>{children}</button>;
}
export function OfficeBadge({ children, state = 'neutral' }: { children: React.ReactNode; state?: 'neutral' | 'good' | 'warning' }) {
  return <span className={'office-badge office-badge--' + state}>{children}</span>;
}
export function OfficeDialog({ title, children, footer, onClose, busy, hidden = false, className = '' }: {
  title: string; children: React.ReactNode; footer?: React.ReactNode; onClose: () => void; busy?: boolean; hidden?: boolean; className?: string;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLElement>(null);
  useEffect(() => {
    if (hidden) return;
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.querySelector<HTMLElement>('input:not([disabled]),textarea:not([disabled]),select:not([disabled]),button:not([disabled])')?.focus();
    return () => { if (previous?.isConnected && !previous.closest('[inert]')) previous.focus(); };
  }, [hidden]);
  return <div hidden={hidden} className='office-dialog-overlay' onKeyDown={event => {
    if (event.key === 'Escape') { event.stopPropagation(); if (!busy) onClose(); }
    if (event.key !== 'Tab') return;
    const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),a[href]') ?? []);
    const first = controls[0], last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }}><section ref={dialog} className={'office-dialog ' + className} role='dialog' aria-modal='true' aria-label={title}>
    <header className='office-dialog__header'><h2>{title}</h2><OfficeButton quiet disabled={busy} aria-label={t('office.actions.close')} onClick={onClose}><Close size={16} /></OfficeButton></header>
    <div className='office-dialog__body'>{children}</div>{footer && <footer className='office-dialog__footer'>{footer}</footer>}
  </section></div>;
}
/** Shared error block: message plus contextual recovery actions supplied by the host. */
export function OfficeErrorPanel({ error, onShowOperation, onResumeFlow, onAuthorize, onOpenUrl }: {
  error: unknown;
  onShowOperation?: (id: string) => void;
  onResumeFlow: (id: string) => void;
  onAuthorize: (scopes: string[]) => void;
  onOpenUrl: (url: string) => void;
}) {
  const { t } = useTranslation();
  if (!error) return null;
  const details = errorDetails(error), original = relatedOperation(error), flow = relatedAuthorization(error);
  const scopes = Array.isArray(details.missing_scopes) ? details.missing_scopes.filter((value): value is string => typeof value === 'string') : [];
  return <div className='office-error' role='alert'><span>{error instanceof Error && error.message.startsWith('office.') ? t(error.message) : officeError(error)}</span><div className='office-actions'>{original && onShowOperation && <OfficeButton quiet onClick={() => onShowOperation(original)}>{t('office.actions.original')}</OfficeButton>}{flow && <OfficeButton quiet onClick={() => onResumeFlow(flow)}>{t('office.auth.resume')}</OfficeButton>}{!!scopes.length && <OfficeButton quiet onClick={() => onAuthorize(scopes)}>{t('office.actions.authorize')}</OfficeButton>}{typeof details.console_url === 'string' && <OfficeButton quiet onClick={() => onOpenUrl(details.console_url as string)}>{t('office.auth.admin')}</OfficeButton>}</div></div>;
}
export interface AuthorizationProps {
  flow: AuthorizationFlow | null; busy: boolean; error?: React.ReactNode;
  onCancel: () => void; onOpen: (url: string) => void; completionHint?: string;
}
export function AuthorizationPanel({ flow, error, busy, onCancel, onOpen, completionHint }: AuthorizationProps) {
  const { t } = useTranslation();
  const active = !!flow && !terminalAuth(flow.phase);
  return <div className='office-authorization'>{error}{flow && <>
    <OfficeBadge state={flow.phase === 'complete' ? 'good' : 'warning'}>{t('office.auth.phase.' + flow.phase)}</OfficeBadge>
    <p>{completionHint && ['complete', 'partial'].includes(flow.phase) ? completionHint : t(flow.phase === 'cancelling' ? 'office.auth.cancellingHint' : flow.phase === 'waiting_admin' ? 'office.auth.adminHint' : terminalAuth(flow.phase) ? 'office.auth.returnHint' : 'office.auth.waitingHint')}</p>
    {flow.message && <p role='status'>{flow.message}</p>}
    {active && flow.phase !== 'cancelling' && flow.verification_url && <OfficeButton primary onClick={() => onOpen(flow.verification_url!)}><LinkOne size={14} />{t('office.auth.open')}</OfficeButton>}
    {flow.console_url && <OfficeButton quiet onClick={() => onOpen(flow.console_url!)}>{t('office.auth.admin')}</OfficeButton>}
    {flow.expires_at && <p className='office-muted office-small'>{t('office.auth.expires', { time: new Date(flow.expires_at).toLocaleString() })}</p>}
    {active && <OfficeButton quiet disabled={busy || flow.phase === 'cancelling'} onClick={onCancel}>{t('office.auth.cancel')}</OfficeButton>}
    {!!flow.missing_scopes.length && <details><summary>{t('office.permissions.exactScopes')}</summary><p className='office-break'>{flow.missing_scopes.join(', ')}</p></details>}
  </>}</div>;
}
export function OperationDialog({ operation, status, busy, error, onClose, onReconcile, onRefresh, onOpen }: {
  operation: OfficeOperation; status: OfficeStatus | null; busy: boolean; error?: React.ReactNode; onClose: () => void; onReconcile: () => void; onRefresh: () => void; onOpen: (url: string) => void;
}) {
  const { t } = useTranslation();
  const sameIdentity = status?.connection.id === operation.connection_id && status.connection.profile === operation.profile && sameActor(operation.actor, status.actor);
  return <OfficeDialog title={t('office.operation.results')} busy={busy} onClose={onClose} className='office-result-dialog' footer={<><span className='office-muted office-small'>{t('office.operation.actor', { name: operation.actor.name || operation.actor.user_id })}</span><div className='office-actions'><OfficeButton onClick={onClose}>{t('office.actions.close')}</OfficeButton><OfficeButton disabled={busy} onClick={onRefresh}>{t('office.actions.refresh')}</OfficeButton>{operation.items.some(pendingItem) && <OfficeButton primary disabled={busy || !sameIdentity || !!error} onClick={onReconcile}>{t('office.actions.reconcile')}</OfficeButton>}</div></>}>
    {error}<div className='office-between'><h3>{t('office.operation.summary.' + operationKind(operation), { count: operation.items.length, defaultValue: t('office.operation.summary.mixed', { count: operation.items.length }) })}</h3><OfficeBadge state={operation.status === 'succeeded' ? 'good' : 'warning'}>{t('office.operation.status.' + operation.status)}</OfficeBadge></div>
    {!sameIdentity && <p className='office-warning'>{t('office.operation.identityChanged')}</p>}
    {operation.items.map(item => <article className='office-receipt' key={item.id}><div className='office-between'><strong>{mutationTitle(item.mutation) || item.target_key}</strong><OfficeBadge state={item.status === 'succeeded' ? 'good' : pendingItem(item) ? 'warning' : 'neutral'}>{t('office.operation.status.' + item.status)}</OfficeBadge></div>
      {item.message && <p>{item.status === 'succeeded' && item.observations.some(observation => observation.kind === 'target_readback') ? t('office.operation.readbackConfirmed') : item.message}</p>}{pendingItem(item) && <p className='office-small office-muted'>{t(item.status === 'unknown' ? 'office.operation.unknownHint' : 'office.operation.baseHint')}</p>}
      {item.remote_url && <OfficeButton quiet onClick={() => onOpen(item.remote_url!)}>{t('office.actions.open')}</OfficeButton>}
      <details><summary>{t('office.operation.evidence')}</summary>{item.attempts.map(attempt => <p key={attempt.id}>{t('office.operation.attempt', { number: attempt.number })} · {t('office.operation.status.' + attempt.status)}{attempt.message && <> · {attempt.message}</>}</p>)}{item.observations.map(observation => <p key={observation.id}>{new Date(observation.created_at).toLocaleString()} · {observation.message}</p>)}</details>
    </article>)}<p className='office-small office-muted office-break'>{operation.id}</p>
  </OfficeDialog>;
}
