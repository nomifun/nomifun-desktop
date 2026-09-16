import { useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './CreativeCanvasChrome.module.css';

export default function CreativeCanvasTitle({ title, disabled, onRename }: {
  title: string;
  disabled?: boolean;
  onRename?: (title: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const editingRef = useRef(false);
  const savingRef = useRef(false);

  useLayoutEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const begin = () => {
    if (disabled || !onRename || savingRef.current) return;
    setDraft(title);
    setError(false);
    editingRef.current = true;
    setEditing(true);
  };
  const cancel = () => {
    editingRef.current = false;
    setEditing(false);
    setError(false);
  };
  const save = async () => {
    if (!editingRef.current || savingRef.current || disabled || !onRename) return;
    const nextTitle = draft.trim();
    if (!nextTitle || nextTitle === title) { cancel(); return; }
    savingRef.current = true;
    setSaving(true);
    setError(false);
    try {
      await onRename(nextTitle);
      cancel();
    } catch {
      setError(true);
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  return <div className={styles.titleEditor}>
    {editing ? <input
      ref={inputRef}
      className={styles.titleInput}
      aria-label={t('creativeStudio.canvases.renamePlaceholder')}
      aria-invalid={error || undefined}
      aria-busy={saving}
      value={draft}
      maxLength={80}
      readOnly={saving || disabled}
      onChange={event => setDraft(event.target.value)}
      onBlur={() => void save()}
      onKeyDown={event => {
        event.stopPropagation();
        if (event.nativeEvent.isComposing) return;
        if (event.key === 'Enter') { event.preventDefault(); void save(); }
        if (event.key === 'Escape' && !savingRef.current) { event.preventDefault(); cancel(); }
      }}
    /> : <h1
      title={onRename ? `${title} · ${t('creativeStudio.canvases.renameCanvas')}` : title}
      tabIndex={onRename && !disabled ? 0 : undefined}
      className={onRename && !disabled ? styles.editableTitle : undefined}
      onDoubleClick={begin}
      onKeyDown={event => {
        if (event.key === 'Enter' || event.key === 'F2') { event.preventDefault(); begin(); }
      }}
    >{title}</h1>}
    {error && <span className={styles.titleError} role='alert'>{t('creativeStudio.canvases.renameFailed')}</span>}
  </div>;
}
