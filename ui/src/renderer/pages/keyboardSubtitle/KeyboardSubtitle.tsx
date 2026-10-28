import { useTranslation } from 'react-i18next';
import { useCallback, useEffect, useLayoutEffect, useRef } from 'react';
import { IconBrush, IconClose, IconCloseCircle, IconDragDotVertical, IconPushpin, IconRecordStop, IconRefresh, IconUndo } from '@arco-design/web-react/icon';
import { documentAction, type KeyboardSelection } from '@/common/keyboard';
import { useKeyboard } from '@/renderer/hooks/keyboard/useKeyboard';
import { useSubtitleEditor } from '@/renderer/hooks/keyboard/useSubtitleEditor';
import { useSubtitleSelection } from '@/renderer/hooks/keyboard/useSubtitleSelection';
import { patchSubtitleText, subtitleTextMap } from './subtitleText';
import './keyboard-subtitle.css';

/** Transparent lyrics window; the 880 × 600 workbench is a separate surface. */
export default function KeyboardSubtitle() {
  const { t } = useTranslation();
  const { state, error, available, busy, action, refresh, getCurrentState } = useKeyboard();
  const glyph = useRef<HTMLSpanElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const commitSelection = useRef<() => KeyboardSelection | null>(() => null);
  const beforeEnd = useCallback(() => commitSelection.current(), []);
  const mayResume = useCallback(() => {
    const node = glyph.current; const current = window.getSelection();
    return Boolean(node && document.hasFocus() && document.activeElement === node && (!current?.rangeCount
      || node.contains(current.anchorNode) && node.contains(current.focusNode)));
  }, []);
  const editor = useSubtitleEditor(state, action, getCurrentState, mayResume, beforeEnd);
  const selection = useSubtitleSelection(state, glyph, content, editor.editable, editor.composing);
  commitSelection.current = selection.forCommit;
  const temporary = useRef<{ span: HTMLSpanElement; original: DocumentFragment } | null>(null);
  const followTail = useRef(true);
  const previousDocument = useRef<string | null>(null);
  const closing = useRef<{ documentId: string | null } | null>(null);
  useEffect(() => {
    if (previousDocument.current !== state?.documentId) followTail.current = true;
    previousDocument.current = state?.documentId ?? null;
    const selection = window.getSelection();
    if (content.current && document.activeElement !== glyph.current && followTail.current && (!selection || selection.isCollapsed)) {
      content.current.scrollTop = content.current.scrollHeight;
    }
  }, [state?.documentId, state?.text, state?.partial, state?.candidate]);
  useEffect(() => {
    const blurred = () => { selection.deactivate(); editor.onBlur(); };
    window.addEventListener('blur', blurred);
    return () => window.removeEventListener('blur', blurred);
  }, [editor.onBlur, selection.deactivate]);

  const phase = state?.phase ?? 'idle';
  const recording = phase === 'recording';
  const finalizing = phase === 'finalizing';
  const capturing = state?.audioState.state === 'starting' || state?.audioState.state === 'capturing';
  const active = recording || finalizing || capturing || state?.audioState.state === 'stopping' || Boolean(state?.capturePreparing);
  const captureStatus = state?.capturePreparing || state?.audioState.state === 'starting' ? 'preparing'
    : state?.audioState.state === 'stopping' || finalizing && state?.audioState.state === 'capturing' ? 'stopping'
      : recording && state?.audioState.state === 'capturing' ? 'listening' : null;
  const text = editor.text ?? state?.text ?? '';
  const hasText = Boolean(text.trim());
  const platformAsr = state?.settings.asrMode === 'platform';
  const partial = !platformAsr ? state?.partial : '';
  const beautifying = phase === 'beautifying';
  // A streamed candidate is presentation only. An active DOM edit/IME lease
  // remains the sole editable source until Rust commits a completed result.
  const preview = beautifying && !state?.error && !state?.editSessionId && editor.text === null && !editor.composing
    ? state?.candidate || null : null;
  const canBeautify = Boolean(state && (active || hasText) && !beautifying);
  const status = captureStatus === 'preparing' ? t('settings.keyboard.recording_preparing')
    : captureStatus === 'stopping' ? t('settings.keyboard.recording_stopping')
    : platformAsr && (recording || finalizing)
      ? t(recording ? 'settings.keyboard.phase_platform_recording' : 'settings.keyboard.phase_platform_finalizing')
      : t(`settings.keyboard.phase_${phase}`);
  const placeholder = !state && available ? t('settings.keyboard.loading')
    : captureStatus ? '' : finalizing ? t('settings.keyboard.lyricsRecognizing')
      : beautifying ? t('settings.keyboard.lyricsBeautifying') : state?.notice || '';
  const failure = (editor.saveError ? t('settings.keyboard.editSaveError') : null)
    || error || state?.error || (state?.audioState.state === 'error' ? state.audioState.detail : null)
    || (!available ? t('settings.keyboard.desktopOnly') : null);
  const invoke = useCallback((value: Parameters<typeof documentAction>[1]) => {
    const current = getCurrentState();
    if (!current) return;
    if (value === 'beautify' || value === 'restore_original') {
      void editor.runAfterSave(current => action(documentAction(current, value)));
    } else {
      if (value === 'close') {
        if (!current.visible || state?.documentId !== current.documentId) return;
        if (closing.current?.documentId === current.documentId) return;
        const request = { documentId: current.documentId };
        closing.current = request;
        selection.clear(); editor.discard();
        void action(documentAction(current, value)).finally(() => {
          if (closing.current === request) closing.current = null;
        });
        return;
      }
      if (value === 'cancel') editor.cancelQueuedAction();
      void action(documentAction(current, value));
    }
  }, [action, editor.cancelQueuedAction, editor.discard, editor.runAfterSave, getCurrentState, selection.clear, state?.documentId]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // This listener belongs only to this subtitle WebView, including its blank
      // background. Standard selection copy and active IME composition stay native.
      if (event.key !== 'Escape' || event.repeat || event.defaultPrevented || event.isComposing
        || event.keyCode === 229 || editor.composing || !document.hasFocus()) return;
      event.preventDefault();
      invoke('close');
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [editor.composing, invoke]);
  useLayoutEffect(() => {
    const node = glyph.current;
    if (!node) return;
    const display = editor.text !== null ? editor.text : preview ?? (text || (!partial ? placeholder : ''));
    node.classList.toggle('keyboard-subtitle__empty', !text && !partial && preview === null && editor.text === null);
    // The browser owns editing/IME markup and its caret until the lease ends.
    if (editor.text !== null) { selection.preserve(); return; }
    if (temporary.current?.span.isConnected) {
      temporary.current.span.before(temporary.current.original); temporary.current.span.remove();
    }
    temporary.current = null;
    patchSubtitleText(node, display);
    if (preview !== null) return;
    if (partial) {
      const map = subtitleTextMap(node);
      const at = state?.dictationSelection;
      const range = document.createRange();
      const start = map.locate(at ? Math.min(at.anchor, at.focus) : display.length);
      const end = map.locate(at ? Math.max(at.anchor, at.focus) : display.length);
      range.setStart(start[0], start[1]); range.setEnd(end[0], end[1]);
      const span = document.createElement('span');
      span.className = 'keyboard-subtitle__partial'; span.setAttribute('aria-label', t('settings.keyboard.partial'));
      span.textContent = partial;
      const original = range.extractContents(); range.insertNode(span);
      temporary.current = { span, original };
    }
    selection.preserve();
  }, [text, preview, partial, placeholder, editor.text, state?.documentId, state?.dictationSelection?.anchor,
    state?.dictationSelection?.focus, selection.preserve, t]);
  const read = () => glyph.current ? subtitleTextMap(glyph.current).text : '';

  return <main className={`keyboard-subtitle${beautifying ? ' keyboard-subtitle--beautifying' : ''}${captureStatus ? ' keyboard-subtitle--capture-status' : ''}`}
    aria-label={t('settings.keyboard.subtitleTitle')}>
    {captureStatus && <div className={`keyboard-subtitle__capture-status keyboard-subtitle__capture-status--${captureStatus}`}
      data-state={captureStatus === 'stopping' ? 'finishing' : captureStatus} aria-hidden='true'>
      {captureStatus === 'listening' ? <span className='keyboard-subtitle__listen-bars' aria-hidden='true'>
        <i /><i /><i /><i /><i />
      </span> : <span className={captureStatus === 'preparing' ? 'keyboard-subtitle__prepare-spinner' : 'keyboard-subtitle__stop-dot'} aria-hidden='true' />}
      <span>{t(`settings.keyboard.recording_${captureStatus}`)}</span>
    </div>}
    {failure && !captureStatus && <div className='keyboard-subtitle__issue'>
      <details key={failure} className='keyboard-subtitle__issue-disclosure'>
        <summary><span role='alert'>{failure}</span></summary>
        <div className='keyboard-subtitle__issue-detail'>{failure}</div>
      </details>
      {available && state && <button type='button' className='keyboard-subtitle__setup'
        disabled={busy} onClick={() => invoke('open_settings')}>{t('settings.keyboard.openSetup')}</button>}
    </div>}
    <div className='keyboard-subtitle__content' ref={content} onScroll={(event) => {
      const node = event.currentTarget;
      followTail.current = node.scrollHeight - node.clientHeight - node.scrollTop < 36;
    }}>
      <div key={state?.documentId ?? 'empty-document'} className='keyboard-subtitle__text' aria-busy={beautifying}
        style={{ fontSize: state?.settings.subtitleFontSize ?? 14 }}>
        <span ref={glyph} className='keyboard-subtitle__glyph' role='textbox' aria-label={t('settings.keyboard.finalText')}
          aria-multiline='true' aria-readonly={!editor.editable} aria-busy={beautifying} tabIndex={0}
          contentEditable={editor.editable ? 'plaintext-only' : false} suppressContentEditableWarning
          onFocus={editor.onFocus} onClick={() => { editor.onFocus(); selection.capture(); }}
          onBlur={() => { selection.deactivate(); editor.onBlur(); }} onMouseUp={selection.capture} onKeyUp={selection.capture}
          onInput={() => { editor.input(read()); selection.capture(); }} onCompositionStart={editor.compositionStart}
          onCompositionEnd={() => { editor.compositionEnd(read()); selection.capture(); }}
          onKeyDown={event => { if ((event.ctrlKey || event.metaKey) && ['b', 'i', 'u'].includes(event.key.toLowerCase())) event.preventDefault(); }} />
        {!beautifying && selection.caret && <span className='keyboard-subtitle__caret' aria-hidden='true'
          style={{ left: selection.caret.left, top: selection.caret.top, height: selection.caret.height }} />}
      </div>
    </div>

    <div className='keyboard-subtitle__toolbar' role='toolbar' aria-label={t('settings.keyboard.subtitleControls')}
      onMouseDown={event => { if ((event.target as HTMLElement).closest('button:not(.keyboard-subtitle__drag)')) event.preventDefault(); }}>
      <button type='button' className='keyboard-subtitle__drag' data-tauri-drag-region
        aria-label={t('settings.keyboard.dragWindow')} title={t('settings.keyboard.dragWindow')}><IconDragDotVertical data-tauri-drag-region /></button>
      {(recording || capturing || state?.capturePreparing) && <button type='button' aria-label={t('settings.keyboard.stop')} title={t('settings.keyboard.stop')}
        onClick={() => {
          const check = state?.microphoneTest;
          if (check?.id && !check.stopped) void action({ action: 'microphone_test_stop', testId: check.id });
          else invoke('stop');
        }}><IconRecordStop /></button>}
      {(active || beautifying) && <button type='button' aria-label={t('settings.keyboard.cancel')} title={t('settings.keyboard.cancel')} onClick={() => invoke('cancel')}><IconCloseCircle /></button>}
      <button type='button' aria-label={t('settings.keyboard.beautify')} title={t('settings.keyboard.beautify')} disabled={!canBeautify} onClick={() => invoke('beautify')}><IconBrush /></button>
      <button type='button' aria-label={t('settings.keyboard.restore')} title={t('settings.keyboard.restore')} disabled={!state || active || phase !== 'idle' || !state.originalText || state.originalText === state.text || busy}
        onClick={() => invoke('restore_original')}><IconUndo /></button>
      <button type='button' aria-label={t('settings.keyboard.alwaysOnTop')} title={t(state?.settings.alwaysOnTop ? 'settings.keyboard.pinned' : 'settings.keyboard.pin')} aria-pressed={state?.settings.alwaysOnTop ?? false}
        disabled={!state || busy} onClick={() => invoke('toggle_pin')}><IconPushpin /></button>
      {editor.saveError && <button type='button' aria-label={t('settings.keyboard.editRetry')} title={t('settings.keyboard.editRetry')} onClick={() => void editor.retry()}><IconRefresh /></button>}
      {failure && !editor.saveError && <button type='button' aria-label={t('settings.keyboard.refresh')} title={t('settings.keyboard.refresh')} onClick={() => void refresh()}><IconRefresh /></button>}
      <button type='button' aria-label={t('settings.keyboard.close')} title={t('settings.keyboard.close')}
        disabled={!state} onClick={() => invoke('close')}><IconClose /></button>
    </div>
    <span className='keyboard-subtitle__sr-only' role='status'>{!captureStatus && editor.saving ? t('settings.keyboard.editSaving') : status}{state?.notice ? ` · ${state.notice}` : ''}</span>
  </main>;
}
