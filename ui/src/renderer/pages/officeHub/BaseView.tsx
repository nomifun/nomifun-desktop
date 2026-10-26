import type { OfficeField, OfficeItem, OfficeMutation, OfficeStatus } from '@/common/types/officeHub';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeButton, OfficeDialog } from './dialogs';
import { baseInputText, editableFieldValue, fieldDisplayText, hasPermission, identityKey, parseBaseLink, selectFieldMode, selectFieldOptions, writeScopes } from './model';
import { WriteFeedback, type FormServices } from './NativeForm';
import { useOfficeQuery } from './useOfficeHub';
import { captureIdentity, useOfficeWrite, type WriteIdentity } from './useOfficeWrite';

const simpleType = (field: OfficeField) => {
  const type = field.field_type.toLowerCase();
  if (selectFieldMode(field) && selectFieldOptions(field).length) return 'select';
  if (['text', 'string', 'multiline_text', 'single_line_text', '1'].includes(type)) return 'text';
  if (['number', 'currency', 'percent', '2'].includes(type)) return 'number';
  if (['checkbox', 'boolean', '7'].includes(type)) return 'checkbox';
  return null;
};
export function BaseView({ status, ownerKey, location, onLocation, authorize, errorPanel, onResultId, refreshKey, onEdit }: {
  status: OfficeStatus | null; ownerKey: string; location: { base_token: string; table_id: string } | null;
  onLocation: (value: { base_token: string; table_id: string }) => void; authorize: () => void; errorPanel: FormServices['errorPanel'];
  onResultId: (id: string) => void; refreshKey: number;
  onEdit: (value: BaseEditSelection) => void;
}) {
  const { t } = useTranslation();
  const [link, setLink] = useState('');
  const [error, setError] = useState<unknown>(null);
  const [offsets, setOffsets] = useState([0]);
  const [selected, setSelected] = useState<string[]>([]);
  const query = useOfficeQuery(location && hasPermission(status, 'base.read') ? { kind: 'base_records', ...location, offset: offsets[offsets.length - 1], filter: null } : null, ownerKey + ':' + refreshKey);
  const available = query.data?.items.filter(item => !item.pending_operation_id) ?? [];
  const records = available.filter(item => selected.includes(item.id));
  useEffect(() => { setSelected([]); setOffsets([0]); }, [location?.base_token, location?.table_id, identityKey(status)]);
  useEffect(() => { if (query.data) setSelected(previous => previous.filter(id => available.some(item => item.id === id))); }, [query.data]);
  const open = () => { try { const value = parseBaseLink(link); setError(null); setOffsets([0]); setSelected([]); onLocation(value); query.refresh(); } catch (reason) { setError(reason); } };
  return <section className='office-content'><form className='office-link-open' onSubmit={event => { event.preventDefault(); open(); }}><input className='office-input' aria-label={t('office.base.link')} placeholder={t('office.base.linkPlaceholder')} value={link} onChange={event => setLink(event.target.value)} /><OfficeButton primary onClick={open}>{t('office.base.open')}</OfficeButton></form>{errorPanel(error)}
    {!hasPermission(status, 'base.read') ? <div className='office-empty'><h3>{t('office.permissions.groups.base')}</h3><p>{t('office.permissions.readMissing')}</p><OfficeButton primary onClick={authorize}>{t('office.actions.authorizeRead')}</OfficeButton></div> : !location ? <div className='office-empty'><h3>{t('office.base.empty')}</h3><p>{t('office.base.linkHint')}</p></div> : <>{errorPanel(query.error)}<div className='office-between office-table-heading'><strong>{t('office.tabs.base')}</strong><div className='office-actions'><span className='office-small office-muted'>{t('office.base.selected', { count: records.length })}</span><OfficeButton disabled={!records.length || records.length > 200 || !captureIdentity(status)} onClick={() => onEdit({ ...location, records, fields: query.data?.fields ?? [], identity: captureIdentity(status)! })}>{t('office.base.batch')}</OfficeButton><OfficeButton quiet onClick={query.refresh}>{t('office.actions.refresh')}</OfficeButton></div></div>
      <div className='office-table-scroll'><table className='office-table office-base-table'><thead><tr><th><input type='checkbox' aria-label={t('office.base.selectAvailable')} checked={!!available.length && records.length === available.length} disabled={!available.length || available.length > 200} onChange={event => setSelected(event.target.checked ? available.map(item => item.id) : [])} /></th>{query.data?.fields.map(field => <th key={field.id}>{field.name}</th>)}<th>{t('office.base.writeState')}</th></tr></thead><tbody>{query.data?.items.map(item => <tr key={item.id}><td><input type='checkbox' aria-label={t('office.base.selectRecord', { title: item.title || item.id })} disabled={!!item.pending_operation_id} checked={selected.includes(item.id)} onChange={event => setSelected(previous => event.target.checked ? [...previous, item.id] : previous.filter(id => id !== item.id))} /></td>{query.data!.fields.map(field => <td key={field.id}>{fieldDisplayText(item.fields[field.id]) || '—'}</td>)}<td>{item.pending_operation_id ? <OfficeButton quiet onClick={() => onResultId(item.pending_operation_id!)}>{t('office.operation.status.' + (item.pending_status ?? 'unknown'))}</OfficeButton> : '—'}</td></tr>)}</tbody></table></div>
      {query.loading && <p role='status'>{t('office.loading')}</p>}{query.data && !query.data.items.length && <p className='office-empty'>{t('office.empty')}</p>}<div className='office-pagination'><OfficeButton disabled={offsets.length === 1 || query.loading} onClick={() => setOffsets(value => value.slice(0,-1))}>{t('office.actions.previous')}</OfficeButton><OfficeButton disabled={!query.data?.has_more || query.loading} onClick={() => setOffsets(value => [...value, value[value.length - 1] + (query.data?.items.length ?? 0)])}>{t('office.actions.more')}</OfficeButton></div>
    </>}
  </section>;
}
export interface BaseEditSelection { base_token: string; table_id: string; records: OfficeItem[]; fields: OfficeField[]; identity: WriteIdentity }
export function BaseEditForm({ selection, ...services }: FormServices & { selection: BaseEditSelection }) {
  const { t } = useTranslation();
  const fields = selection.fields.filter(field => field.writable && simpleType(field));
  const [fieldId, setFieldId] = useState(fields[0]?.id ?? '');
  const field = fields.find(item => item.id === fieldId);
  const [value, setValue] = useState(field ? baseInputText(field, selection.records[0]?.fields[field.id]) : '');
  const [error, setError] = useState<unknown>(null);
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  const changed = selection.identity.key !== identityKey(services.status);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; services.onSaved(write.result); } }, [write.result, services.onSaved]);
  const save = async () => {
    try {
      if (!field || changed || !selection.records.length || selection.records.length > 200) throw new Error('office.validation.batch');
      const next = editableFieldValue(field, value);
      const items: OfficeMutation[] = selection.records.map(record => ({ kind: 'base_update', client_item_id: record.id, base_token: selection.base_token, table_id: selection.table_id, record_id: record.id, fields: { [field.id]: next }, before: { [field.id]: record.fields[field.id] ?? null } }));
      const scopes = writeScopes(services.status, items);
      if (scopes.length) { services.authorize(scopes); return; }
      setError(null); await write.save(items, selection.identity);
    } catch (reason) { setError(reason); }
  };
  const type = field && simpleType(field);
  return <OfficeDialog title={t('office.base.batchTitle', { count: selection.records.length })} onClose={services.onClose} hidden={services.hidden} busy={write.loading} footer={<><span className='office-small office-muted'>{t('office.operation.actor', { name: selection.identity.actor.name })}</span><div className='office-actions'><OfficeButton disabled={write.loading} onClick={services.onClose}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={!field || write.loading || write.locked || changed} onClick={() => void save()}>{t('office.actions.save')}</OfficeButton></div></>}>
    {changed && <p className='office-warning'>{t('office.operation.identityChangedInput')}</p>}{services.errorPanel(error)}<WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
    <fieldset className='office-fieldset' disabled={write.loading || write.locked || changed}><label className='office-label'>{t('office.base.field')}<select className='office-input' value={fieldId} onChange={event => { setFieldId(event.target.value); const next = fields.find(item => item.id === event.target.value); setValue(next ? baseInputText(next, selection.records[0]?.fields[next.id]) : ''); }}>{fields.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
      {field && <label className='office-label'>{t('office.base.value')}{type === 'select' ? <select className='office-input' multiple={selectFieldMode(field) === 'multiple'} value={selectFieldMode(field) === 'multiple' ? JSON.parse(value || '[]') : (JSON.parse(value || '[]')[0] ?? '')} onChange={event => setValue(JSON.stringify(Array.from(event.target.selectedOptions).map(item => item.value)))}><option value=''>{t('office.base.chooseValue')}</option>{selectFieldOptions(field).map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select> : type === 'checkbox' ? <input type='checkbox' checked={value === 'true'} onChange={event => setValue(String(event.target.checked))} /> : <input className='office-input' type={type === 'number' ? 'number' : 'text'} step={type === 'number' ? 'any' : undefined} value={value} onChange={event => setValue(event.target.value)} />}</label>}
      {!fields.length && <p className='office-muted'>{t('office.base.noWritable')}</p>}<p className='office-muted office-small'>{t('office.base.simpleFields')}</p>
      {field && <div className='office-table-scroll'><table className='office-table'><thead><tr><th>{t('office.base.record')}</th><th>{t('office.base.before')}</th><th>{t('office.base.after')}</th></tr></thead><tbody>{selection.records.map(record => <tr key={record.id}><td>{record.title || record.id}</td><td>{fieldDisplayText(record.fields[field.id]) || '—'}</td><td>{type === 'select' ? fieldDisplayText(JSON.parse(value || '[]')) : value || '—'}</td></tr>)}</tbody></table></div>}
    </fieldset>
  </OfficeDialog>;
}
