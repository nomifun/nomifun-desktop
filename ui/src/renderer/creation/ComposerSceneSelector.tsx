import { useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import {
  autoUpdate, flip, FloatingFocusManager, FloatingPortal, offset, safePolygon, shift,
  useClick, useDismiss, useFloating, useHover, useInteractions, useListNavigation, useRole,
} from '@floating-ui/react';
import { AddPicture, CheckSmall, CloseSmall, Down, MessageOne, Music, Star, VideoTwo } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useCreationComposer } from './CreationComposerContext';
import styles from './ComposerSceneSelector.module.css';

const scenes = ['chat', 'image', 'video', 'music'] as const;
const icons = { chat: MessageOne, image: AddPicture, video: VideoTwo, music: Music };
const HINT_KEY = 'nomifun.composer.scene-discovery.v1';
const hintListeners = new Set<() => void>();
let memoryDismissed = false;
const readHintDismissed = () => {
  try { return localStorage.getItem(HINT_KEY) === 'true'; } catch { return memoryDismissed; }
};
const subscribeHint = (listener: () => void) => {
  hintListeners.add(listener);
  const onStorage = (event: StorageEvent) => { if (!event.key || event.key === HINT_KEY) listener(); };
  window.addEventListener('storage', onStorage);
  return () => { hintListeners.delete(listener); window.removeEventListener('storage', onStorage); };
};
function dismissHint() {
  try { localStorage.setItem(HINT_KEY, 'true'); } catch { memoryDismissed = true; }
  hintListeners.forEach(listener => listener());
}

/** Shared scene navigation stays beside the Agent while configuration changes below. */
export function ComposerSceneHeader({ agent }: { agent?: ReactNode }) {
  const creation = useCreationComposer();
  if (!agent && !creation) return null;
  return <div className={styles.header} data-composer-scene-header>
    {agent && <div className={styles.agent}>{agent}</div>}
    {creation && <ComposerSceneSelector />}
  </div>;
}

export function SceneDiscoveryHint() {
  const creation = useCreationComposer();
  const dismissed = useSyncExternalStore(subscribeHint, readHintDismissed, () => true);
  const { t } = useTranslation();
  if (!creation) return null;
  // Keep this small row stable when the one-time hint is dismissed.
  return <div className={styles.hintSlot}>
    {!dismissed && <div className={styles.hint}>
      <Star size={14} aria-hidden />
      <span>{t('creation.scene.discovery')}</span>
      <button type='button' onClick={dismissHint} aria-label={t('creation.scene.dismissHint')}><CloseSmall size={14} /></button>
    </div>}
  </div>;
}

export default function ComposerSceneSelector() {
  const creation = useCreationComposer();
  return creation ? <SceneSelector creation={creation} /> : null;
}

function SceneSelector({ creation }: { creation: NonNullable<ReturnType<typeof useCreationComposer>> }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const [hoverOpened, setHoverOpened] = useState(false);
  const listRef = useRef<Array<HTMLButtonElement | null>>([]);
  const selected = creation.draft.mode ?? 'chat';
  const selectedIndex = scenes.indexOf(selected);
  const Icon = icons[selected];
  const disabled = Boolean(creation.preparing);
  const { refs, floatingStyles, context } = useFloating({
    open, onOpenChange(next, _event, reason) {
      setOpen(next);
      if (next) setHoverOpened(reason === 'hover');
      else { setActiveIndex(null); dismissHint(); }
    },
    placement: 'bottom-start', strategy: 'fixed', whileElementsMounted: autoUpdate,
    middleware: [offset(10), flip({ padding: 12 }), shift({ padding: 12 })],
  });
  const { getReferenceProps, getFloatingProps, getItemProps } = useInteractions([
    useHover(context, { enabled: !disabled, delay: { open: 210, close: 240 }, handleClose: safePolygon({ buffer: 8 }) }),
    useClick(context, { enabled: !disabled }),
    useDismiss(context),
    useRole(context, { role: 'menu' }),
    useListNavigation(context, { listRef, activeIndex, selectedIndex: hoverOpened ? null : selectedIndex, onNavigate: setActiveIndex, cols: 2, orientation: 'both', loop: true, focusItemOnHover: false }),
  ]);
  const choose = (scene: typeof scenes[number]) => {
    if (scene !== selected) {
      if (scene === 'chat') creation.exit();
      else creation.selectMode(scene);
    }
    setOpen(false);
    setActiveIndex(null);
    dismissHint();
  };
  return <>
    <button type='button' ref={refs.setReference} className={styles.trigger} disabled={disabled}
      data-testid='composer-scene-selector'
      aria-label={t('creation.scene.trigger', { scene: t(`creation.mode.${selected}`) })}
      {...getReferenceProps()}>
      <Icon size={16} aria-hidden />
      <span className={styles.label}>{t(`creation.mode.${selected}`)}</span>
      <span className={styles.preview} aria-hidden>
        {scenes.filter(scene => scene !== selected).map(scene => { const Preview = icons[scene]; return <Preview key={scene} size={14} />; })}
      </span>
      <Down size={12} aria-hidden />
    </button>
    {open && <FloatingPortal><FloatingFocusManager context={context} modal={false} initialFocus={hoverOpened ? -1 : 0}>
      <div ref={refs.setFloating} className={styles.menu} style={floatingStyles} aria-label={t('creation.scene.title')} {...getFloatingProps()}>
        <div className={styles.menuTitle}>{t('creation.scene.title')}</div>
        <div className={styles.options} role='presentation'>
          {scenes.map((scene, index) => {
            const SceneIcon = icons[scene];
            return <button type='button' key={scene} ref={node => { listRef.current[index] = node; }}
              className={styles.option} role='menuitemradio' aria-checked={selected === scene}
              tabIndex={activeIndex === index ? 0 : -1} data-creation-mode={scene}
              {...getItemProps({ onClick: () => choose(scene) })}>
              <span className={styles.optionTitle}><SceneIcon size={16} aria-hidden /><span>{t(`creation.mode.${scene}`)}</span><CheckSmall size={16} className={styles.check} aria-hidden /></span>
              <span className={styles.description}>{t(`creation.scene.description.${scene}`)}</span>
            </button>;
          })}
        </div>
      </div>
    </FloatingFocusManager></FloatingPortal>}
  </>;
}
