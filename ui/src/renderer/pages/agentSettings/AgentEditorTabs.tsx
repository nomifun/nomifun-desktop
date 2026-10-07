import { useRef } from 'react';
import styles from './AgentSettingsPage.module.css';

type Tab = { key: string; label: string };

/** Roving-focus desktop tabs: Arrow keys move focus and activate the destination. */
export default function AgentEditorTabs({
  tabs,
  active,
  onChange,
  idPrefix,
  label,
}: {
  tabs: readonly Tab[];
  active: string;
  onChange: (key: string) => void;
  idPrefix: string;
  label: string;
}) {
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const activateAt = (index: number) => {
    const target = tabs[(index + tabs.length) % tabs.length];
    onChange(target.key);
    requestAnimationFrame(() => buttons.current.get(target.key)?.focus());
  };
  return (
    <nav className={styles.editorTabs} role='tablist' aria-label={label}>
      {tabs.map((tab, index) => (
        <button
          key={tab.key}
          ref={(node) => {
            if (node) buttons.current.set(tab.key, node);
            else buttons.current.delete(tab.key);
          }}
          type='button'
          role='tab'
          tabIndex={active === tab.key ? 0 : -1}
          aria-selected={active === tab.key}
          aria-controls={`${idPrefix}-panel-${tab.key}`}
          id={`${idPrefix}-tab-${tab.key}`}
          className={active === tab.key ? styles.activeTab : ''}
          onClick={() => onChange(tab.key)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowRight') {
              event.preventDefault();
              activateAt(index + 1);
            } else if (event.key === 'ArrowLeft') {
              event.preventDefault();
              activateAt(index - 1);
            } else if (event.key === 'Home') {
              event.preventDefault();
              activateAt(0);
            } else if (event.key === 'End') {
              event.preventDefault();
              activateAt(tabs.length - 1);
            }
          }}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
