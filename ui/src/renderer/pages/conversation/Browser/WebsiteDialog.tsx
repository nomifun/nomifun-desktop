import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { BrowserCommand, BrowserDialog } from './client';
import styles from './BrowserWorkspacePanel.module.css';

type Props = { dialog: BrowserDialog; locked: boolean; busy: boolean; onReply: (command: BrowserCommand) => void };

/** A tab-local website dialog. It never makes the conversation itself modal. */
export default function WebsiteDialog({ dialog, locked, busy, onReply }: Props) {
  const { t } = useTranslation();
  const title = useId();
  const message = useId();
  const input = useRef<HTMLInputElement>(null);
  const acceptButton = useRef<HTMLButtonElement>(null);
  // Omitted text preserves the browser's full default, even if its display was
  // truncated. Never silently replace that default with the visible prefix.
  const [edited, setEdited] = useState<string | null>(null);
  useEffect(() => {
    if (!locked) (dialog.kind === 'prompt' ? input.current : acceptButton.current)?.focus();
  }, [dialog.request_id, dialog.kind, locked]);
  const reply = (accept: boolean) => {
    if (locked || busy) return;
    onReply({ command: 'dialog', target: dialog.target, request_id: dialog.request_id, accept,
      ...(accept && dialog.kind === 'prompt' && edited !== null ? { text: edited } : {}) });
  };
  return <div className={styles.websiteDialogBackdrop}>
    <form className={styles.websiteDialog} role='dialog' aria-labelledby={title} aria-describedby={message}
      onSubmit={event => { event.preventDefault(); reply(true); }}
      onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); reply(false); } }}>
      <h3 id={title}>{t('browserWorkspace.dialogTitle')}</h3>
      <div className={styles.dialogOrigin}>{dialog.origin === 'null' ? t('browserWorkspace.dialogUnknownOrigin') : dialog.origin}</div>
      <p id={message}>{dialog.kind === 'before_unload' ? t('browserWorkspace.dialogLeaveWarning') : dialog.message}</p>
      {dialog.text_truncated && <p className={styles.dialogHint}>{t('browserWorkspace.dialogTruncated')}</p>}
      {dialog.kind === 'prompt' && <label>{t('browserWorkspace.dialogInput')}
        <input ref={input} value={edited ?? dialog.default_text} maxLength={65536} disabled={locked || busy}
          onChange={event => setEdited(event.target.value)} autoComplete='off' />
      </label>}
      {locked ? <div className={styles.dialogHint} role='status'>{t('browserWorkspace.dialogAgentHandling')}</div>
        : <div className={styles.dialogActions}>
          {dialog.kind !== 'alert' && <button type='button' disabled={busy} onClick={() => reply(false)}>{t(dialog.kind === 'before_unload' ? 'browserWorkspace.dialogStay' : 'browserWorkspace.dialogCancel')}</button>}
          <button ref={acceptButton} type='submit' disabled={busy}>{t(dialog.kind === 'before_unload' ? 'browserWorkspace.dialogLeave' : 'browserWorkspace.dialogAccept')}</button>
        </div>}
    </form>
  </div>;
}
