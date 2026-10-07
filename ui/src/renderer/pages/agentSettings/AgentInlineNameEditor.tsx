import { Edit } from '@icon-park/react';
import React, { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './AgentSettingsPage.module.css';

type Props = {
  value: string;
  fallback: string;
  disabled?: boolean;
  onChange: (value: string) => void;
};

const AgentInlineNameEditor: React.FC<Props> = ({
  value,
  fallback,
  disabled = false,
  onChange,
}) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [pending, setPending] = useState(value);
  const cancelBlur = useRef(false);
  const displayName = value.trim() || fallback;

  const startEditing = () => {
    if (disabled) return;
    cancelBlur.current = false;
    setPending(value);
    setEditing(true);
  };
  const commit = () => {
    const next = pending.trim();
    if (next && next !== value) onChange(next);
    setEditing(false);
  };
  const cancel = () => {
    cancelBlur.current = true;
    setPending(value);
    setEditing(false);
  };

  if (editing) {
    return <div className={styles.headerNameLine}>
      <input
        autoFocus
        type='text'
        className={styles.headerNameInput}
        value={pending}
        maxLength={80}
        aria-label={t('agentSettings.fields.name')}
        onChange={(event) => setPending(event.currentTarget.value)}
        onBlur={() => {
          if (cancelBlur.current) {
            cancelBlur.current = false;
            return;
          }
          commit();
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
          if (event.key === 'Enter') {
            event.preventDefault();
            event.currentTarget.blur();
          } else if (event.key === 'Escape') {
            event.preventDefault();
            cancel();
          }
        }}
      />
    </div>;
  }

  return <div className={styles.headerNameLine}>
    <h2 className={styles.headerName} onClick={startEditing} title={t('agentSettings.workbench.editName')}>
      {displayName}
    </h2>
    <button
      type='button'
      className={styles.headerNameEdit}
      disabled={disabled}
      aria-label={t('agentSettings.workbench.editName')}
      title={t('agentSettings.workbench.editName')}
      onClick={startEditing}
    >
      <Edit theme='outline' size={13} />
    </button>
  </div>;
};

export default AgentInlineNameEditor;
