import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { Alert, Button, Input, Spin, Tag } from '@arco-design/web-react';
import NomiPagination from '@/renderer/components/base/NomiPagination';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { Application, Plus, Refresh } from '@icon-park/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import { appErrorMessage } from './appHostingModel';
import styles from './AppsPage.module.css';

export default function AppsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data: apps = [], error: loadError, isLoading, mutate } = useSWR('apps.hosted', () => appHosting.list.invoke(), { refreshInterval: 2000 });
  const [search, setSearch] = useState('');
  const [tag, setTag] = useState<string>();
  const [page, setPage] = useState(1);
  const tags = useMemo(() => [...new Set(apps.flatMap((app) => app.tags ?? []))].sort(), [apps]);
  const filtered = useMemo(() => apps.filter((app) => (!tag || app.tags?.includes(tag)) &&
    `${app.name} ${app.description ?? ''} ${(app.tags ?? []).join(' ')}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())), [apps, search, tag]);
  const currentPage = Math.min(page, Math.max(1, Math.ceil(filtered.length / 9)));
  const enter = (app: HostedApp) => void navigate(`/apps/${encodeURIComponent(app.id)}`);
  return <main className={styles.page}>
    <PageHeader className={styles.header} title={<>{t('appHosting.title')}<span className={styles.titleSeparator}>/</span>{t('appHosting.home.title')}</>} actions={<>
      <Button type='primary' icon={<Plus fill='currentColor' />} onClick={() => void navigate('/apps/new')}>{t('appHosting.create.action')}</Button>
      <Button onClick={() => void navigate('/apps/import')}>{t('appHosting.import.action')}</Button>
      <Input.Search className={styles.appSearch} value={search} onChange={(value) => { setSearch(value); setPage(1); }} allowClear
        placeholder={t('appHosting.home.search')} aria-label={t('appHosting.home.search')} />
      <Button aria-label={t('appHosting.actions.refresh')} icon={<Refresh />} onClick={() => void mutate()} />
    </>} />
    <section className={styles.categories} aria-label={t('appHosting.home.categories')}>
      <span>{t('appHosting.home.categories')}</span><Button type={!tag ? 'primary' : 'text'} size='small' onClick={() => { setTag(undefined); setPage(1); }}>{t('appHosting.home.all')}</Button>
      {tags.map((value) => <Button key={value} size='small' type={tag === value ? 'primary' : 'text'} onClick={() => { setTag(value); setPage(1); }}>{value}</Button>)}
    </section>
    {loadError && <Alert type='error' content={appErrorMessage(loadError)} />}
    {isLoading && <Spin />}
    {!isLoading && apps.length === 0 && <section className={styles.empty}><Application size={40} theme='outline' />
      <h2>{t('appHosting.empty.title')}</h2><p>{t('appHosting.empty.hint')}</p>
      <div className={styles.actions}><Button type='primary' onClick={() => void navigate('/apps/new')}>{t('appHosting.create.action')}</Button>
        <Button onClick={() => void navigate('/apps/import')}>{t('appHosting.import.action')}</Button></div>
    </section>}
    {!isLoading && apps.length > 0 && filtered.length === 0 && <p className={styles.empty}>{t('appHosting.home.noResults')}</p>}
    <section className={styles.list} aria-label={t('appHosting.list')}>
      {filtered.slice((currentPage - 1) * 9, currentPage * 9).map((app) => <article key={app.id} className={styles.card}>
          <button className={styles.cardLink} onClick={() => enter(app)} aria-label={t('appHosting.home.edit', { name: app.name })}>
            <div className={styles.cardHeading}><Application size={18} theme='outline' /><h2 title={app.name}>{app.name}</h2>
              <div className={styles.meta}><span className={styles.cardStatus} data-state={app.state} title={app.last_error || undefined}>{t(`appHosting.state.${app.state}`)}</span>
                {app.operation ? <span role='status' title={t(`appHosting.operation.${app.operation}`, { defaultValue: app.operation })}>{t(`appHosting.operation.${app.operation}`, { defaultValue: app.operation })}</span>
                  : app.preview && <span>{t('appHosting.preview')}</span>}</div>
            </div>
            <p>{app.description || t('appHosting.home.noDescription')}</p>
            <div className={styles.cardTags} aria-label={(app.tags ?? []).join(', ')}>{(app.tags ?? []).slice(0, 2).map((value) => <Tag key={value} size='small'>{value}</Tag>)}
              {(app.tags ?? []).length > 2 && <span title={app.tags.join(', ')}>+{app.tags.length - 2}</span>}</div>
          </button>
        </article>)}
    </section>
    {filtered.length > 0 && <footer className={styles.pagination}><NomiPagination current={currentPage} pageSize={9} total={filtered.length} onChange={setPage} /></footer>}
  </main>;
}
