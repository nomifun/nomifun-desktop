import type { AppProjectSession } from '@/common/types/appHosting';
import { Alert, Button, Spin, Tag } from '@arco-design/web-react';
import { Add, Down } from '@icon-park/react';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { isAppProjectSessionBusy } from './appHostingModel';
import styles from './AppsPage.module.css';

type Props = {
  items: AppProjectSession[];
  selectedSessionId: string | null | undefined;
  selectedName?: string;
  total: number;
  loading: boolean;
  hasMore: boolean;
  error?: string;
  newSessionDisabled?: boolean;
  onSelect: (id: string) => void;
  onOpenConversation: (id: string) => void;
  onNewSession: () => void;
  onLoadMore: () => void;
  onRefresh: () => void;
};

/** The selector navigates retained ordinary Sessions; it never copies their messages. */
export default function AppProjectHistory({ items, selectedSessionId, selectedName, total, loading, hasMore, error,
  newSessionDisabled, onSelect, onOpenConversation, onNewSession, onLoadMore, onRefresh }: Props) {
  const { t, i18n } = useTranslation();
  const disclosure = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const outside = (event: PointerEvent) => {
      if (disclosure.current?.open && event.target instanceof Node && !disclosure.current.contains(event.target)) disclosure.current.open = false;
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !disclosure.current?.open) return;
      disclosure.current.open = false; disclosure.current.querySelector('summary')?.focus(); event.preventDefault();
    };
    document.addEventListener('pointerdown', outside); document.addEventListener('keydown', escape);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape); };
  }, []);
  const selected = items.find(item => String(item.conversation.id) === selectedSessionId);
  const select = (id: string, editable: boolean) => {
    if (disclosure.current) disclosure.current.open = false;
    if (editable) onSelect(id); else onOpenConversation(id);
  };
  return <div className={styles.projectHistory}>
    <div className={styles.projectHistoryToolbar}>
      <details ref={disclosure} className={styles.projectHistoryDisclosure}>
        <summary aria-label={t('appHosting.project.history')}>
          <span>{t('appHosting.project.history')}<small>{total}</small></span>
          <strong>{selectedSessionId === null ? t('appHosting.project.newSession') : selectedName || selected?.conversation.name || t('appHosting.project.chooseSession')}</strong>
          <Down size={14} aria-hidden />
        </summary>
        <div className={styles.projectHistoryMenu}>
          <p className={styles.muted}>{t('appHosting.project.historyHint')}</p>
          {error && <Alert type='warning' content={error} />}
          {loading && items.length === 0 ? <Spin /> : items.length === 0 && <p className={styles.muted}>{t('appHosting.project.empty')}</p>}
          <ul aria-label={t('appHosting.project.history')}>
            {items.map(({ conversation, editable_in_app, archived }) => <li key={conversation.id}>
              <button type='button' aria-current={String(conversation.id) === selectedSessionId ? 'true' : undefined}
                onClick={() => select(String(conversation.id), editable_in_app)}>
                <span className={styles.projectHistoryName}>{conversation.name || t('appHosting.project.untitled')}
                  {archived && <Tag size='small'>{t('appHosting.project.archived')}</Tag>}
                  {isAppProjectSessionBusy(conversation) && <Tag size='small' color='green'>{t(conversation.extra.execution_phase === 'paused' ? 'appHosting.project.paused' : 'appHosting.project.processing')}</Tag>}
                </span>
                <span className={styles.projectHistoryMeta}>
                  <time dateTime={new Date(conversation.modified_at).toISOString()}>{new Intl.DateTimeFormat(i18n.language, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(conversation.modified_at)}</time>
                  {!editable_in_app && <span>{t('appHosting.project.openConversation')}</span>}
                </span>
              </button>
            </li>)}
          </ul>
          <div className={styles.actions}>
            {hasMore && <Button size='small' loading={loading} onClick={onLoadMore}>{t('appHosting.project.loadMore')}</Button>}
            <Button type='text' size='small' aria-label={t('appHosting.project.refresh')} onClick={onRefresh}>{t('appHosting.actions.refresh')}</Button>
          </div>
        </div>
      </details>
      <Button size='small' icon={<Add />} disabled={newSessionDisabled} onClick={onNewSession}>{t('appHosting.project.newSession')}</Button>
    </div>
    {error && <p role='status' className={styles.projectHistoryWarning}>{t('appHosting.project.refreshFailed')}</p>}
  </div>;
}
