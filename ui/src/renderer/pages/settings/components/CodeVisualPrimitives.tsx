/**
 * Adapted from codevisual-share's mini-panel, section-components, browser-tabs,
 * command-palette, health-check, file-explorer and state-empty components.
 * Presentation demos become real controls here: app theme tokens, accessible
 * semantics and live data replace sample values, skeletons and looping effects.
 */
import classNames from 'classnames';
import { Check, Close, Right, Search } from '@icon-park/react';
import React, { useId, useRef, useState } from 'react';
import './settings-workspace.css';

export function VisualPanel({ title, description, action, children, className, label }: {
  title?: React.ReactNode; description?: string; action?: React.ReactNode;
  children: React.ReactNode; className?: string; label?: string;
}) {
  const titleId = useId();
  return <section className={classNames('cv-panel', className)} aria-label={label} aria-labelledby={label || !title ? undefined : titleId}>
      {(title || description || action) && <header className='cv-panel__header'>
        <div>{title && <h2 id={titleId}>{title}</h2>}{description && <p>{description}</p>}</div>
        {action}
      </header>}
      <div className='cv-panel__body'>{children}</div>
  </section>;
}

export function VisualRow({ label, description, children, disabled = false }: {
  label: React.ReactNode; description?: React.ReactNode; children?: React.ReactNode; disabled?: boolean;
}) {
  return <div className={classNames('cv-row', { 'cv-row--disabled': disabled })}>
    <div className='cv-row__copy'><div className='cv-row__label'>{label}</div>{description && <p>{description}</p>}</div>
    {children && <div className='cv-row__control'>{children}</div>}
  </div>;
}

export function VisualSwitch({ label, checked, disabled = false, onChange }: {
  label: string; checked: boolean; disabled?: boolean; onChange: (checked: boolean) => void | Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  return <button type='button' role='switch' aria-label={label} aria-checked={checked} aria-busy={busy}
    disabled={disabled || busy} className='cv-switch' onClick={async () => {
      setBusy(true);
      try { await onChange(!checked); } finally { setBusy(false); }
    }}><span className='cv-switch__thumb' /></button>;
}

export type VisualTabItem = { key: string; label: string; icon?: React.ReactNode; dot?: boolean };

export function VisualTabs({ items, activeKey, onChange, label, id }: {
  items: VisualTabItem[]; activeKey: string; onChange: (key: string) => void; label: string; id: string;
}) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  return <div className='cv-tabs' role='tablist' aria-label={label}>
    {items.map((item, index) => <button key={item.key} type='button' role='tab'
      id={`${id}-tab-${item.key}`} aria-controls={`${id}-panel`} aria-selected={item.key === activeKey}
      tabIndex={item.key === activeKey ? 0 : -1} ref={(node) => { refs.current[index] = node; }}
      onClick={() => onChange(item.key)} onKeyDown={(event) => {
        let next = index;
        if (event.key === 'ArrowRight') next = (index + 1) % items.length;
        else if (event.key === 'ArrowLeft') next = (index - 1 + items.length) % items.length;
        else if (event.key === 'Home') next = 0;
        else if (event.key === 'End') next = items.length - 1;
        else return;
        event.preventDefault(); onChange(items[next].key); refs.current[next]?.focus();
      }}>
      {item.icon}<span>{item.label}</span>{item.dot && <span className='cv-tabs__dot' aria-hidden='true' />}
    </button>)}
  </div>;
}

export function VisualChoice<T extends string>({ label, value, options, onChange, disabled = false }: {
  label: string; value: T; options: Array<{ value: T; label: string }>; onChange: (value: T) => void; disabled?: boolean;
}) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  return <div className='cv-choice' role='radiogroup' aria-label={label} aria-disabled={disabled}>
    {options.map((option, index) => <button key={option.value} type='button' role='radio'
      aria-checked={option.value === value} disabled={disabled} tabIndex={option.value === value ? 0 : -1}
      ref={(node) => { refs.current[index] = node; }} onClick={() => onChange(option.value)}
      onKeyDown={(event) => {
        let next = index;
        if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (index + 1) % options.length;
        else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (index - 1 + options.length) % options.length;
        else return;
        event.preventDefault(); onChange(options[next].value); refs.current[next]?.focus();
      }}>
      <span className='cv-choice__check' aria-hidden='true'>{option.value === value && <Check theme='outline' size={12} />}</span><span>{option.label}</span>
    </button>)}
  </div>;
}

export function VisualSearch({ value, onChange, label, clearLabel }: {
  value: string; onChange: (value: string) => void; label: string; clearLabel: string;
}) {
  const ref = useRef<HTMLInputElement>(null);
  return <div className='cv-search'>
    <Search theme='outline' size={16} />
    <input ref={ref} type='search' value={value} aria-label={label} placeholder={label}
      onChange={(event) => onChange(event.target.value)} onKeyDown={(event) => {
        if (event.key === 'Escape') { event.preventDefault(); onChange(''); }
      }} />
    {value && <button type='button' aria-label={clearLabel} onClick={() => { onChange(''); ref.current?.focus(); }}>
      <Close theme='outline' size={13} />
    </button>}
  </div>;
}

export type VisualTone = 'success' | 'warning' | 'danger' | 'info' | 'neutral';
export function VisualStatus({ children, tone = 'neutral' }: { children: React.ReactNode; tone?: VisualTone }) {
  return <span className={`cv-status cv-status--${tone}`}><span className='cv-status__dot' aria-hidden='true' />{children}</span>;
}

export function VisualListRow({ icon, title, description, meta, action, onClick }: {
  icon: React.ReactNode; title: React.ReactNode; description?: React.ReactNode;
  meta?: React.ReactNode; action?: React.ReactNode; onClick?: () => void;
}) {
  const content = <>
    <span className='cv-list-row__icon' aria-hidden='true'>{icon}</span>
    <span className='cv-list-row__copy'><span className='cv-list-row__title'>{title}</span>
      {description && <span className='cv-list-row__description'>{description}</span>}
      {meta && <span className='cv-list-row__meta'>{meta}</span>}
    </span>
    <span className='cv-list-row__action'>{action}{onClick && <Right theme='outline' size={15} />}</span>
  </>;
  return onClick
    ? <button type='button' className='cv-list-row cv-list-row--link' onClick={onClick}>{content}</button>
    : <div className='cv-list-row'>{content}</div>;
}

export function VisualEmpty({ icon, title, description, action }: {
  icon: React.ReactNode; title: string; description?: string; action?: React.ReactNode;
}) {
  return <div className='cv-empty' role='status'>
    <span className='cv-empty__icon' aria-hidden='true'>{icon}</span><h2>{title}</h2>
    {description && <p>{description}</p>}{action}
  </div>;
}
