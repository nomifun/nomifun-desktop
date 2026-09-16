import { ipcBridge } from '@/common';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import CompanionAvatar from '@/renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@/renderer/pages/companion/characters/customMeta';
import { useCompanions } from '@/renderer/pages/nomi/useNomi';
import { Message } from '@arco-design/web-react';
import { Add, CloseSmall, Down, More, Right, Search, Up } from '@icon-park/react';
import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import podium from '@/renderer/assets/images/companion-podium.png';
import { fitShowcaseFigure, showcaseCapacity, visibleCompanions } from './companionShowcaseLayout';
import GuidPopover from './GuidPopover';
import styles from './GuidCompanionShowcase.module.css';

const CreateCompanionModal = lazy(() => import('@/renderer/pages/nomi/CompanionSidebar/CreateCompanionModal'));

const COLLAPSED_KEY = 'nomifun.home.companions.collapsed';
const readCollapsed = () => {
  try { return localStorage.getItem(COLLAPSED_KEY) === 'true'; } catch { return false; }
};

export type GuidCompanionShowcaseProps = {
  companions: ICompanionWithStatus[];
  loading?: boolean;
  error?: Error | null;
  openingId?: CompanionId | null;
  onRetry: () => void;
  onCreate: () => void;
  onManage: (id?: CompanionId) => void;
  onOpenChat: (companion: ICompanionWithStatus) => void;
};

/** No sample companions are injected: every displayed figure belongs to the user's roster. */
export function GuidCompanionShowcaseView({ companions, loading, error, openingId, onRetry, onCreate, onManage, onOpenChat }: GuidCompanionShowcaseProps) {
  const { t } = useTranslation();
  const { ref, width } = useContainerWidth<HTMLElement>();
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [bodyHeight, setBodyHeight] = useState<number>();

  // Measure natural content, including wrapped avatars and shorter desktop windows.
  // The outer height animates; the measured inner content never inherits that height.
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!body) return;
    const measure = () => setBodyHeight(body.getBoundingClientRect().height);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(body);
    return () => observer.disconnect();
  }, [collapsed, loading, error, companions.length, width]);
  const [selectedId, setSelectedId] = useState<CompanionId | null>(null);
  const [detailId, setDetailId] = useState<CompanionId | null>(null);
  const [allOpen, setAllOpen] = useState(false);
  const [rosterMenuId, setRosterMenuId] = useState<CompanionId | null>(null);
  const [query, setQuery] = useState('');
  const capacity = showcaseCapacity(width);
  const visible = visibleCompanions(companions, capacity, selectedId);
  const slotWidth = Math.max(40, ((width || 360) - 24 - (visible.length - 1) * 16) / Math.max(visible.length, 1));

  useEffect(() => {
    if (!companions.some((item) => item.companion_id === selectedId)) {
      setSelectedId(null);
      setDetailId(null);
    }
  }, [companions, selectedId]);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    setDetailId(null);
    try { localStorage.setItem(COLLAPSED_KEY, String(next)); } catch { /* in-memory preference still works */ }
  };
  const matches = companions.filter((item) => item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const avatar = (companion: ICompanionWithStatus, size: number, full = false) => <CompanionAvatar
    character={companion.character} companionId={companion.companion_id}
    customFigure={customFigureMetaOf(companion)} size={size}
    displayMode={full ? 'full' : 'auto'} mood='content' activity='idle'
  />;
  const chooseFromList = (companion: ICompanionWithStatus) => {
    setSelectedId(companion.companion_id);
    setAllOpen(false);
    setQuery('');
    // The selected figure is promoted into the visible set before its popover opens.
    setDetailId(companion.companion_id);
  };

  return <section ref={ref} className={styles.showcase} aria-label={t('guid.showcase.title')} data-collapsed={collapsed}>
    <header className={styles.header}>
      <h1>{t('guid.showcase.title')}<span className={styles.count}>{companions.length > 0 ? `· ${companions.length}` : ''}</span></h1>
      <div className={styles.headerActions}>
        {companions.length > 0 && <>
          <GuidPopover open={allOpen} onOpenChange={(open) => { setAllOpen(open); setQuery(''); setDetailId(null); setRosterMenuId(null); }}
            label={t('guid.showcase.all', { count: companions.length })}
            triggerClassName={styles.textButton} panelClassName={styles.rosterPanel} initialFocus={1}
            trigger={<>{t('guid.showcase.all', { count: companions.length })}<Right size={13} /></>}>
            <div className={styles.rosterHeader}>
              <h2>{t('guid.showcase.all', { count: companions.length })}</h2>
              <button type='button' className={styles.iconButton} aria-label={t('guid.showcase.closeList')}
                onClick={() => { setAllOpen(false); setRosterMenuId(null); }}><CloseSmall size={17} /></button>
            </div>
            <div className={styles.search}>
              <Search size={16} />
              <input aria-label={t('guid.showcase.search')} placeholder={t('guid.showcase.search')}
                value={query} onChange={(event) => { setQuery(event.target.value); setRosterMenuId(null); }} />
            </div>
            <div className={styles.roster}>
              <div className={styles.rosterGrid} style={{ gridTemplateRows: `repeat(${Math.max(1, Math.ceil(matches.length / 2))}, 36px)` }}>
              {matches.map((companion) => <div className={styles.rosterRow}
                data-selected={selectedId === companion.companion_id} key={companion.companion_id}>
                <button type='button' className={styles.rosterChoice} aria-pressed={selectedId === companion.companion_id}
                  onClick={() => { setRosterMenuId(null); chooseFromList(companion); }}>
                  <span className={styles.rosterAvatar}>{avatar(companion, 28)}</span>
                  <span className={styles.rosterName} title={companion.name}>{companion.name}</span>
                </button>
                <GuidPopover open={rosterMenuId === companion.companion_id}
                  onOpenChange={(open) => setRosterMenuId(open ? companion.companion_id : null)}
                  label={t('guid.showcase.moreFor', { name: companion.name })}
                  trigger={<More size={15} />} triggerClassName={styles.iconButton}
                  panelClassName={styles.rosterActions} portal={false}>
                  <strong>{companion.name}</strong>
                  <button type='button' disabled={Boolean(openingId)} onClick={() => { setRosterMenuId(null); setAllOpen(false); onOpenChat(companion); }}>{t('guid.showcase.openChat')}</button>
                  <button type='button' onClick={() => { setRosterMenuId(null); setAllOpen(false); onManage(companion.companion_id); }}>{t('guid.showcase.manage')}</button>
                </GuidPopover>
              </div>)}
              </div>
              {matches.length === 0 && <p className={styles.notice}>{t('guid.showcase.noMatches')}</p>}
            </div>
            <div className={styles.rosterFooter}>
              <button type='button' className={styles.textButton} onClick={() => { setAllOpen(false); onCreate(); }}><Add size={14} />{t('guid.showcase.create')}</button>
              <button type='button' className={styles.textButton} onClick={() => onManage()}>{t('guid.showcase.manage')}</button>
            </div>
          </GuidPopover>
          <button type='button' className={styles.textButton} aria-expanded={!collapsed} onClick={toggleCollapsed}>
            {collapsed ? t('guid.showcase.expand') : t('guid.showcase.collapse')}{collapsed ? <Down size={13} /> : <Up size={13} />}
          </button>
        </>}
      </div>
    </header>
    {error && <div role='alert' className={styles.error}>{t('guid.showcase.loadFailed')}
      <button type='button' className={styles.textButton} onClick={onRetry}>{t('guid.showcase.retry')}</button>
    </div>}
    <div className={styles.bodyTransition} style={{ height: bodyHeight }}>
    <div ref={bodyRef}>
    {loading && companions.length === 0 ? <div role='status' className={styles.empty}>{t('guid.showcase.loading')}</div>
      : companions.length === 0 ? !error && <div className={styles.empty}>
        <button type='button' className={styles.createFigure} onClick={onCreate} aria-label={t('guid.showcase.create')}>
          <Add size={30} /><img src={podium} alt='' className={styles.emptyPodium} />
        </button>
        <div className={styles.emptyCopy}><h2>{t('guid.showcase.emptyTitle')}</h2><p>{t('guid.showcase.emptyDescription')}</p>
          <button type='button' className={styles.primaryButton} onClick={onCreate}>{t('guid.showcase.create')}</button>
        </div>
      </div> : <div key={collapsed ? 'compact' : 'expanded'} className={collapsed ? styles.compactStage : styles.stage}
        style={{ gridTemplateColumns: `repeat(${visible.length}, minmax(0, 1fr))` }}>
        {visible.map((companion) => {
          const meta = customFigureMetaOf(companion);
          const figureHeight = fitShowcaseFigure(meta?.aspect ?? 1, Math.min(slotWidth - 18, 210), meta ? 216 : 136);
          const selected = selectedId === companion.companion_id;
          return <GuidPopover key={`${companion.companion_id}-${collapsed}`} open={detailId === companion.companion_id}
            onOpenChange={(open) => { setDetailId(open ? companion.companion_id : null); if (open) setSelectedId(companion.companion_id); }}
            label={companion.name} pressed={selected} placement={collapsed ? 'top' : 'right-start'} anchorToFigure={!collapsed}
            panelClassName={styles.detail} triggerClassName={collapsed ? styles.compactCompanion : styles.companion}
            trigger={<>
              <span className={collapsed ? styles.smallAvatar : styles.figureSlot}>
                <span data-showcase-art className={styles.figureArtwork}>{avatar(companion, collapsed ? 36 : figureHeight, !collapsed)}</span>
                {!collapsed && <img src={podium} alt='' className={styles.podium} />}
              </span>
              <span className={styles.companionName} title={companion.name}>{companion.name}</span>
            </>}>
            <strong className={styles.detailName}>{companion.name}</strong>
            <button type='button' className={styles.primaryButton} disabled={Boolean(openingId)}
              onClick={() => onOpenChat(companion)}>
              {openingId === companion.companion_id ? t('guid.showcase.opening') : t('guid.showcase.openChat')}<Right size={14} />
            </button>
            <button type='button' className={styles.textButton} onClick={() => onManage(companion.companion_id)}>{t('guid.showcase.manage')}</button>
          </GuidPopover>;
        })}
      </div>}
    </div>
    </div>
  </section>;
}

export default function GuidCompanionShowcase() {
  const roster = useCompanions();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const [createOpen, setCreateOpen] = useState(false);
  const [openingId, setOpeningId] = useState<CompanionId | null>(null);
  const opening = useRef(false);
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const manage = (id?: CompanionId) => {
    void navigate(id ? `/nomi?companion=${encodeURIComponent(id)}&tab=overview` : '/nomi');
  };
  const openChat = async (companion: ICompanionWithStatus) => {
    if (opening.current) return;
    const configured = companion.status ? companion.status.model_configured : companion.model !== null;
    if (!configured) { Message.info(t('nomi.chat.modelMissing')); manage(companion.companion_id); return; }
    opening.current = true;
    setOpeningId(companion.companion_id);
    try {
      const thread = await ipcBridge.companion.ensureCompanionSession.invoke({ companion_id: companion.companion_id });
      if (active.current) void navigate(`/conversation/${thread.conversation_id}`);
    } catch {
      if (active.current) Message.error(t('guid.showcase.chatFailed'));
    } finally {
      opening.current = false;
      if (active.current) setOpeningId(null);
    }
  };
  return <>
    <GuidCompanionShowcaseView {...roster} openingId={openingId} onRetry={() => void roster.refresh()}
      onCreate={() => setCreateOpen(true)} onManage={manage} onOpenChat={(companion) => void openChat(companion)} />
    {createOpen && <Suspense fallback={null}><CreateCompanionModal visible onCancel={() => setCreateOpen(false)} onCreated={(profile) => manage(profile.companion_id)} /></Suspense>}
  </>;
}
