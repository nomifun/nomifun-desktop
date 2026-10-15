/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './RobotCapabilitiesModal.module.css';

const isObject = (value: unknown): value is Record<string, unknown> =>
  value != null && typeof value === 'object' && !Array.isArray(value);

interface JsonValueProps {
  id: string;
  value: unknown;
  objectOnly?: boolean;
  onChange: (value: unknown) => void;
  onValidityChange: (valid: boolean) => void;
}

function JsonValue({ id, value, objectOnly, onChange, onValidityChange }: JsonValueProps) {
  const { t } = useTranslation();
  const [text, setText] = useState(() => JSON.stringify(value === undefined ? {} : value, null, 2));
  const [invalid, setInvalid] = useState(false);
  const previousValue = useRef(value);
  useEffect(() => {
    if (previousValue.current === value) return;
    previousValue.current = value;
    setText(JSON.stringify(value === undefined ? {} : value, null, 2));
    setInvalid(false);
    onValidityChange(true);
  }, [value, onValidityChange]);
  useEffect(() => () => onValidityChange(true), [onValidityChange]);
  return <>
    <textarea id={id} className={styles.json} value={text} aria-invalid={invalid}
      onChange={(event) => {
        const next = event.target.value;
        setText(next);
        try {
          const parsed: unknown = JSON.parse(next);
          if (objectOnly && !isObject(parsed)) throw new Error('object required');
          setInvalid(false);
          onValidityChange(true);
          onChange(parsed);
        } catch {
          setInvalid(true);
          onValidityChange(false);
        }
      }} />
    {invalid && <span className={styles.error} role='alert'>
      {t(objectOnly ? 'nomi.robot.capabilities.jsonObjectInvalid' : 'nomi.robot.capabilities.jsonInvalid')}
    </span>}
  </>;
}

interface ArgumentFieldProps {
  name: string;
  schema: Record<string, unknown>;
  value: unknown;
  included: boolean;
  required: boolean;
  onChange: (value: unknown, included: boolean) => void;
  onValidityChange: (valid: boolean) => void;
}

function ArgumentField({ name, schema, value, included, required, onChange, onValidityChange }: ArgumentFieldProps) {
  const { t } = useTranslation();
  const id = useId();
  const [numericText, setNumericText] = useState(typeof value === 'number' ? String(value) : '');
  const [numericInvalid, setNumericInvalid] = useState(false);
  const previous = useRef({ value, included });
  useEffect(() => {
    if (previous.current.value === value && previous.current.included === included) return;
    previous.current = { value, included };
    setNumericText(typeof value === 'number' ? String(value) : '');
    setNumericInvalid(false);
    onValidityChange(true);
  }, [value, included, onValidityChange]);
  useEffect(() => () => onValidityChange(true), [onValidityChange]);
  const enumeration = Array.isArray(schema.enum) ? schema.enum : null;
  const type = typeof schema.type === 'string' ? schema.type : null;
  const initialValue = schema.default ?? enumeration?.[0] ??
    (type === 'string' ? '' : type === 'number' || type === 'integer' ? 0 : type === 'boolean' ? false : type === 'array' ? [] : {});
  return <div className={styles.parameter}>
    <div className={styles.parameterHeader}>
      <label htmlFor={id}><code>{name}</code>{required ? ' *' : ''}</label>
      <label className={styles.inlineLabel}>
        <input type='checkbox' checked={included} onChange={(event) => {
          onValidityChange(true);
          onChange(included ? undefined : initialValue, event.target.checked);
        }} />
        {t('nomi.robot.capabilities.includeArgument')}
      </label>
    </div>
    {typeof schema.description === 'string' && <span className={styles.hint}>{schema.description}</span>}
    {included && (enumeration ? (
      <select id={id} className={styles.input} value={JSON.stringify(value)}
        onChange={(event) => onChange(JSON.parse(event.target.value), true)}>
        {!enumeration.some((option) => JSON.stringify(option) === JSON.stringify(value)) &&
          <option value={JSON.stringify(value)}>{JSON.stringify(value)}</option>}
        {enumeration.map((option, index) => <option key={index} value={JSON.stringify(option)}>
          {typeof option === 'string' ? option : JSON.stringify(option)}
        </option>)}
      </select>
    ) : type === 'boolean' ? (
      <label className={styles.inlineLabel} htmlFor={id}>
        <input id={id} type='checkbox' checked={value === true}
          onChange={(event) => onChange(event.target.checked, true)} />
        {t('nomi.robot.capabilities.booleanTrue')}
      </label>
    ) : type === 'number' || type === 'integer' ? (
      <input id={id} className={styles.input} type='number' value={numericText} aria-invalid={numericInvalid}
        min={typeof schema.minimum === 'number' ? schema.minimum : undefined}
        max={typeof schema.maximum === 'number' ? schema.maximum : undefined}
        step={type === 'integer' ? 1 : typeof schema.multipleOf === 'number' ? schema.multipleOf : 'any'}
        onChange={(event) => {
          setNumericText(event.target.value);
          const number = event.target.valueAsNumber;
          const valid = Number.isFinite(number) && (type !== 'integer' || Number.isInteger(number));
          setNumericInvalid(!valid);
          onValidityChange(valid);
          if (valid) onChange(number, true);
        }} />
    ) : type === 'string' ? (
      <input id={id} className={styles.input} value={typeof value === 'string' ? value : ''}
        minLength={typeof schema.minLength === 'number' ? schema.minLength : undefined}
        maxLength={typeof schema.maxLength === 'number' ? schema.maxLength : undefined}
        onChange={(event) => onChange(event.target.value, true)} />
    ) : <JsonValue id={id} value={value} onChange={(next) => onChange(next, true)}
      onValidityChange={onValidityChange} />)}
  </div>;
}

interface RobotCapabilityArgumentsProps {
  schema: Record<string, unknown>;
  value: Record<string, unknown>;
  onChange: (value: Record<string, unknown>) => void;
  onValidityChange: (key: string, valid: boolean) => void;
}

/** Parameters only edit a draft profile; this component never calls device tools. */
export default function RobotCapabilityArguments({ schema, value, onChange, onValidityChange }: RobotCapabilityArgumentsProps) {
  const { t } = useTranslation();
  const id = useId();
  const properties = isObject(schema.properties) ? schema.properties : null;
  const required = Array.isArray(schema.required) ? schema.required : [];
  // Stable callbacks keep invalid JSON edits intact across parent renders.
  const validityCallbacks = React.useMemo(() => new Map<string, (valid: boolean) => void>(), [onValidityChange]);
  const validityFor = (key: string) => {
    if (!validityCallbacks.has(key)) validityCallbacks.set(key, (valid) => onValidityChange(key, valid));
    return validityCallbacks.get(key)!;
  };
  if (!properties) return <div className={styles.parameter}>
    <label htmlFor={id}>{t('nomi.robot.capabilities.argumentsJson')}</label>
    <JsonValue id={id} value={value} objectOnly onChange={(next) => onChange(next as Record<string, unknown>)}
      onValidityChange={validityFor('$')} />
  </div>;
  const names = [...new Set([...Object.keys(properties), ...Object.keys(value)])];
  return <div className={styles.parameters}>
    {names.length === 0 && <span className={styles.hint}>{t('nomi.robot.capabilities.noArguments')}</span>}
    {names.map((name) => <ArgumentField key={name} name={name}
      schema={isObject(properties[name]) ? properties[name] : {}} value={value[name]}
      included={Object.prototype.hasOwnProperty.call(value, name)} required={required.includes(name)}
      onValidityChange={validityFor(name)} onChange={(next, included) => {
        const updated = { ...value };
        if (included) updated[name] = next;
        else delete updated[name];
        onChange(updated);
      }} />)}
  </div>;
}
