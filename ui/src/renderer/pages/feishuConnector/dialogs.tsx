import type { AuthorizationFlow, FeishuOperation, FeishuStatus } from '@/common/types/feishuConnector';
import { Close, LinkOne } from '@icon-park/react';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { errorDetails, mutationTitle, feishuError, operationKind, pendingItem, relatedAuthorization, relatedOperation, sameActor, terminalAuth } from './model';

export function FeishuButton({ children, primary, quiet, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean; quiet?: boolean }) {
  return <button type='button' {...props} className={['feishu-button', primary ? 'feishu-button--primary' : '', quiet ? 'feishu-button--quiet' : '', props.className ?? ''].join(' ')}>{children}</button>;
}
export function FeishuBadge({ children, state = 'neutral' }: { children: React.ReactNode; state?: 'neutral' | 'good' | 'warning' }) {
  return <span className={'feishu-badge feishu-badge--' + state}>{children}</span>;
}
export function FeishuDialog({ title, children, footer, onClose, busy, hidden = false, className = '' }: {
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
  return <div hidden={hidden} className='feishu-dialog-overlay' onKeyDown={event => {
    if (event.key === 'Escape') { event.stopPropagation(); if (!busy) onClose(); }
    if (event.key !== 'Tab') return;
    const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),a[href]') ?? []);
    const first = controls[0], last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }}><section ref={dialog} className={'feishu-dialog ' + className} role='dialog' aria-modal='true' aria-label={title}>
    <header className='feishu-dialog__header'><h2>{title}</h2><FeishuButton quiet disabled={busy} aria-label={t('feishu.actions.close')} onClick={onClose}><Close size={16} /></FeishuButton></header>
    <div className='feishu-dialog__body'>{children}</div>{footer && <footer className='feishu-dialog__footer'>{footer}</footer>}
  </section></div>;
}
/** Shared error block: message plus contextual recovery actions supplied by the host. */
export function FeishuErrorPanel({ error, onShowOperation, onResumeFlow, onAuthorize, onOpenUrl }: {
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
  return <div className='feishu-error' role='alert'><span>{error instanceof Error && error.message.startsWith('feishu.') ? t(error.message) : feishuError(error)}</span><div className='feishu-actions'>{original && onShowOperation && <FeishuButton quiet onClick={() => onShowOperation(original)}>{t('feishu.actions.original')}</FeishuButton>}{flow && <FeishuButton quiet onClick={() => onResumeFlow(flow)}>{t('feishu.auth.resume')}</FeishuButton>}{!!scopes.length && <FeishuButton quiet onClick={() => onAuthorize(scopes)}>{t('feishu.actions.authorize')}</FeishuButton>}{typeof details.console_url === 'string' && <FeishuButton quiet onClick={() => onOpenUrl(details.console_url as string)}>{t('feishu.auth.admin')}</FeishuButton>}</div></div>;
}
export interface AuthorizationProps {
  flow: AuthorizationFlow | null; busy: boolean; error?: React.ReactNode;
  onCancel: () => void; onOpen: (url: string) => void; completionHint?: string;
}
export function AuthorizationPanel({ flow, error, busy, onCancel, onOpen, completionHint }: AuthorizationProps) {
  const { t } = useTranslation();
  const active = !!flow && !terminalAuth(flow.phase);
  return <div className='feishu-authorization'>{error}{flow && <>
    <FeishuBadge state={flow.phase === 'complete' ? 'good' : 'warning'}>{t('feishu.auth.phase.' + flow.phase)}</FeishuBadge>
    <p>{completionHint && ['complete', 'partial'].includes(flow.phase) ? completionHint : t(flow.phase === 'cancelling' ? 'feishu.auth.cancellingHint' : flow.phase === 'waiting_admin' ? 'feishu.auth.adminHint' : terminalAuth(flow.phase) ? 'feishu.auth.returnHint' : 'feishu.auth.waitingHint')}</p>
    {flow.message && <p role='status'>{flow.message}</p>}
    {active && flow.phase !== 'cancelling' && flow.verification_url && <FeishuButton primary onClick={() => onOpen(flow.verification_url!)}><LinkOne size={14} />{t('feishu.auth.open')}</FeishuButton>}
    {flow.console_url && <FeishuButton quiet onClick={() => onOpen(flow.console_url!)}>{t('feishu.auth.admin')}</FeishuButton>}
    {flow.expires_at && <p className='feishu-muted feishu-small'>{t('feishu.auth.expires', { time: new Date(flow.expires_at).toLocaleString() })}</p>}
    {active && <FeishuButton quiet disabled={busy || flow.phase === 'cancelling'} onClick={onCancel}>{t('feishu.auth.cancel')}</FeishuButton>}
    {!!flow.missing_scopes.length && <details><summary>{t('feishu.permissions.exactScopes')}</summary><p className='feishu-break'>{flow.missing_scopes.join(', ')}</p></details>}
  </>}</div>;
}
export function OperationDialog({ operation, status, busy, error, onClose, onReconcile, onRefresh, onOpen }: {
  operation: FeishuOperation; status: FeishuStatus | null; busy: boolean; error?: React.ReactNode; onClose: () => void; onReconcile: () => void; onRefresh: () => void; onOpen: (url: string) => void;
}) {
  const { t } = useTranslation();
  const sameIdentity = status?.connection.id === operation.connection_id && status.connection.profile === operation.profile && sameActor(operation.actor, status.actor);
  return <FeishuDialog title={t('feishu.operation.results')} busy={busy} onClose={onClose} className='feishu-result-dialog' footer={<><span className='feishu-muted feishu-small'>{t('feishu.operation.actor', { name: operation.actor.name || operation.actor.user_id })}</span><div className='feishu-actions'><FeishuButton onClick={onClose}>{t('feishu.actions.close')}</FeishuButton><FeishuButton disabled={busy} onClick={onRefresh}>{t('feishu.actions.refresh')}</FeishuButton>{operation.items.some(pendingItem) && <FeishuButton primary disabled={busy || !sameIdentity || !!error} onClick={onReconcile}>{t('feishu.actions.reconcile')}</FeishuButton>}</div></>}>
    {error}<div className='feishu-between'><h3>{t('feishu.operation.summary.' + operationKind(operation), { count: operation.items.length, defaultValue: t('feishu.operation.summary.mixed', { count: operation.items.length }) })}</h3><FeishuBadge state={operation.status === 'succeeded' ? 'good' : 'warning'}>{t('feishu.operation.status.' + operation.status)}</FeishuBadge></div>
    {!sameIdentity && <p className='feishu-warning'>{t('feishu.operation.identityChanged')}</p>}
    {operation.items.map(item => <article className='feishu-receipt' key={item.id}><div className='feishu-between'><strong>{mutationTitle(item.mutation) || item.target_key}</strong><FeishuBadge state={item.status === 'succeeded' ? 'good' : pendingItem(item) ? 'warning' : 'neutral'}>{t('feishu.operation.status.' + item.status)}</FeishuBadge></div>
      {item.message && <p>{item.status === 'succeeded' && item.observations.some(observation => observation.kind === 'target_readback') ? t('feishu.operation.readbackConfirmed') : item.message}</p>}{pendingItem(item) && <p className='feishu-small feishu-muted'>{t(item.status === 'unknown' ? 'feishu.operation.unknownHint' : 'feishu.operation.baseHint')}</p>}
      {item.remote_url && <FeishuButton quiet onClick={() => onOpen(item.remote_url!)}>{t('feishu.actions.open')}</FeishuButton>}
      <details><summary>{t('feishu.operation.evidence')}</summary>{item.attempts.map(attempt => <p key={attempt.id}>{t('feishu.operation.attempt', { number: attempt.number })} · {t('feishu.operation.status.' + attempt.status)}{attempt.message && <> · {attempt.message}</>}</p>)}{item.observations.map(observation => <p key={observation.id}>{new Date(observation.created_at).toLocaleString()} · {observation.message}</p>)}</details>
    </article>)}<p className='feishu-small feishu-muted feishu-break'>{operation.id}</p>
  </FeishuDialog>;
}
