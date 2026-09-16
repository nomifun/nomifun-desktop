import { useRef, useSyncExternalStore, type ReactNode } from 'react';
import { AddPicture, CloseSmall, MessageOne, Music, Star, VideoTwo } from '@icon-park/react';
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
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);
  const selected = creation.draft.mode ?? 'chat';
  const disabled = Boolean(creation.preparing);
  const choose = (scene: typeof scenes[number]) => {
    // Always restore the general assistant, even when a stale draft already
    // presents chat as selected while its Agent is still creative/custom.
    if (scene === 'chat') creation.exit();
    else if (scene !== selected) creation.selectMode(scene);
    dismissHint();
  };
  return <div className={styles.scenes} role='group' aria-label={t('creation.scene.title')} data-testid='composer-scene-selector'>
    {scenes.map((scene, index) => {
      const Icon = icons[scene];
      return <button key={scene} type='button' ref={node => { buttons.current[index] = node; }}
        className={styles.sceneButton} disabled={disabled}
        aria-label={t(`creation.mode.${scene}`)} aria-pressed={selected === scene}
        data-creation-mode={scene} onClick={() => choose(scene)}
        onKeyDown={event => {
          const next = event.key === 'ArrowRight' ? (index + 1) % scenes.length
            : event.key === 'ArrowLeft' ? (index + scenes.length - 1) % scenes.length
            : event.key === 'Home' ? 0 : event.key === 'End' ? scenes.length - 1 : null;
          if (next === null) return;
          event.preventDefault();
          event.stopPropagation();
          buttons.current[next]?.focus();
        }}>
        <span className={styles.sceneIcon} aria-hidden><Icon size={16} fill='currentColor' /></span>
        <span className={styles.sceneLabel} aria-hidden>{t(`creation.mode.${scene}`)}</span>
      </button>;
    })}
  </div>;
}
