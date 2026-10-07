import { useEffect, useId, useMemo, useState } from 'react';
import { Checkbox, Input, InputNumber, Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { parsePluginParameterObject, pluginParameterFields, pluginParameterValuesRepresentable, setPluginParameterValue } from './pluginParameterModel';
import styles from './PluginParameterFields.module.css';

export default function PluginParameterFields({ schema, value, onChange, disabled = false, jsonLabel }: {
  schema: unknown;
  value: string;
  onChange: (json: string) => void;
  disabled?: boolean;
  jsonLabel?: string;
}) {
  const { t } = useTranslation();
  const id = useId();
  const fields = useMemo(() => pluginParameterFields(schema), [schema]);
  const values = parsePluginParameterObject(value);
  const root = schema && typeof schema === 'object' && !Array.isArray(schema) ? schema as Record<string, unknown> : {};
  const representable = fields !== null && values !== null && pluginParameterValuesRepresentable(fields, values)
    && !(root.additionalProperties === false && Object.keys(values).some(key => !fields.some(field => field.key === key)));
  const needsJSON = !representable;
  const [jsonExpanded, setJsonExpanded] = useState(needsJSON);
  useEffect(() => { if (needsJSON) setJsonExpanded(true); }, [needsJSON]);
  const description = typeof root.description === 'string' ? root.description : undefined;
  return <div className={styles.parameters}>
    {description && <p className={styles.description}>{description}</p>}
    {representable && fields.length > 0 && <div className={styles.fields}>
      {fields.map((field, index) => {
        const current = values[field.key];
        const present = Object.hasOwn(values, field.key);
        const optional = !field.required && <span className={styles.optional}>{t('pluginPlatform.config.optional')}</span>;
        const required = field.required && <span className={styles.required} aria-hidden='true'>*</span>;
        const hint = field.description ? `${id}-${index}-hint` : undefined;
        const change = (next: string | number | boolean | undefined) => onChange(setPluginParameterValue(value, field, next));
        return <div key={field.key} className={styles.field}>
          {field.type === 'boolean' && !field.options ? <div className={styles.booleanRow}>
            <Checkbox aria-label={field.label} aria-describedby={hint} disabled={disabled} checked={current === true}
              onChange={checked => change(checked)}>{field.label}{required}</Checkbox>{optional}
            {!field.required && present && <button type='button' disabled={disabled} className={styles.clear}
              aria-label={t('pluginPlatform.config.clearParameter', { name: field.label })} onClick={() => change(undefined)}>{t('pluginPlatform.config.clear')}</button>}
            {!present && <span className={styles.unset}>{t('pluginPlatform.config.notSet')}</span>}
          </div> : <>
            <label htmlFor={`${id}-${index}`} className={styles.label}>{field.label}{required}{optional}</label>
            {field.options ? <Select id={`${id}-${index}`} aria-label={field.label} aria-describedby={hint} disabled={disabled} allowClear={!field.required}
              value={present ? String(field.options.indexOf(current as string | number | boolean)) : undefined}
              placeholder={t('pluginPlatform.config.chooseValue')} onChange={selected => change(selected === undefined ? undefined : field.options![Number(selected)])}
              options={field.options.map((option, index) => ({ value: String(index), label: typeof option === 'boolean'
                ? t(option ? 'pluginPlatform.config.yes' : 'pluginPlatform.config.no') : String(option) }))} />
              : field.type === 'string' ? <Input id={`${id}-${index}`} aria-label={field.label} aria-describedby={hint} disabled={disabled}
                value={typeof current === 'string' ? current : ''} onChange={change} allowClear
                placeholder={field.defaultValue === undefined ? undefined : String(field.defaultValue)} />
                : <InputNumber id={`${id}-${index}`} aria-label={field.label} aria-describedby={hint} disabled={disabled}
                  value={typeof current === 'number' ? current : undefined} step={field.type === 'integer' ? 1 : .1}
                  onChange={next => change(typeof next === 'number' && Number.isFinite(next) ? next : undefined)}
                  placeholder={field.defaultValue === undefined ? undefined : String(field.defaultValue)} />}
          </>}
          {field.description && <p id={hint} className={styles.hint}>{field.description}</p>}
        </div>;
      })}
    </div>}
    {representable && fields.length === 0 && <p className={styles.description}>{t('pluginPlatform.config.noParameters')}</p>}
    <details className={styles.json} open={jsonExpanded} onToggle={event => setJsonExpanded(event.currentTarget.open)}>
      <summary>{t('pluginPlatform.config.editJson')}</summary>
      <label className={styles.field}><span className={styles.label}>{jsonLabel ?? t('pluginPlatform.config.values')}</span>
        <Input.TextArea className={styles.editor} value={value} onChange={next => { setJsonExpanded(true); onChange(next); }}
          disabled={disabled} spellCheck={false} aria-label={jsonLabel ?? t('pluginPlatform.config.values')} autoSize={{ minRows: 4, maxRows: 9 }} />
      </label>
      <details className={styles.schema}><summary>{t('pluginPlatform.config.schema')}</summary><pre>{JSON.stringify(schema, null, 2)}</pre></details>
    </details>
  </div>;
}
