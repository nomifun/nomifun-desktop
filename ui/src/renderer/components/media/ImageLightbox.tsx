import { useEffect, useLayoutEffect, useState } from 'react';
import { Button, Message, Modal, Tooltip } from '@arco-design/web-react';
import { Close, Download, Left, Minus, Plus, Right } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import styles from './ImageLightbox.module.css';

export interface ImageLightboxNavigation {
  index: number;
  total: number;
  onIndexChange(index: number): void;
}

export default function ImageLightbox({
  src,
  title,
  alt = title,
  loading = false,
  error = null,
  onClose,
  onRetry,
  onSaveAs,
  navigation,
  zIndex,
}: {
  src?: string | null;
  title: string;
  alt?: string;
  loading?: boolean;
  error?: string | null;
  onClose(): void;
  onRetry?: () => void;
  onSaveAs?: () => Promise<unknown>;
  navigation?: ImageLightboxNavigation;
  /** Raise the lightbox above product-local overlay stacks such as the canvas composer. */
  zIndex?: number;
}) {
  const { t } = useTranslation();
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  const [bounds, setBounds] = useState({ width: 1, height: 1 });
  const [natural, setNatural] = useState({ width: 0, height: 0 });
  const [scale, setScale] = useState<number | null>(null);
  const [imageFailed, setImageFailed] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [saving, setSaving] = useState(false);
  const hasNavigation = Boolean(navigation && navigation.total > 1);

  useEffect(() => {
    const modal = viewport?.closest<HTMLElement>('.arco-modal');
    if (!modal || !navigation || navigation.total <= 1) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const direction = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0;
      if (!direction) return;
      event.preventDefault();
      event.stopPropagation();
      const index = navigation.index + direction;
      if (index >= 0 && index < navigation.total) navigation.onIndexChange(index);
    };
    modal.addEventListener('keydown', onKeyDown);
    return () => modal.removeEventListener('keydown', onKeyDown);
  }, [navigation, viewport]);

  useLayoutEffect(() => {
    setNatural({ width: 0, height: 0 });
    setScale(null);
    setImageFailed(false);
  }, [src]);

  useEffect(() => {
    const element = viewport;
    if (!element) return;
    const measure = () => setBounds({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [viewport]);
  const fit = natural.width ? Math.min(1, (bounds.width - 32) / natural.width, (bounds.height - 32) / natural.height) : 1;
  const zoom = scale ?? Math.max(.01, fit);
  const failed = Boolean(error) || imageFailed;
  const loaded = Boolean(src) && natural.width > 0 && !failed;
  const saveAs = async () => {
    if (!onSaveAs || saving) return;
    setSaving(true);
    try { await onSaveAs(); }
    catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setSaving(false); }
  };
  const retry = () => {
    setNatural({ width: 0, height: 0 });
    setScale(null);
    setImageFailed(false);
    setReloadKey((current) => current + 1);
    onRetry?.();
  };
  const previewTitle = t('common.imagePreview.title', { defaultValue: '查看图片' });
  const saveAsLabel = t('common.imagePreview.saveAs', { defaultValue: '图片另存为' });
  const closeLabel = t('common.imagePreview.close', { defaultValue: '关闭图片预览' });
  const fitLabel = t('common.imagePreview.fit', { defaultValue: '适应窗口' });
  const previousLabel = t('common.imagePreview.previous', { defaultValue: '上一张图片' });
  const nextLabel = t('common.imagePreview.next', { defaultValue: '下一张图片' });
  return <Modal visible title={previewTitle} aria-label={previewTitle} className={`nomifun-modal-fullscreen ${styles.modal}`} wrapClassName={styles.wrap} maskStyle={{ background: 'rgba(0, 0, 0, .88)', zIndex }} wrapStyle={zIndex === undefined ? undefined : { zIndex }} footer={null} closable={false} focusLock unmountOnExit onCancel={onClose}>
    <div className={styles.viewport} ref={setViewport} aria-busy={loading || (!loaded && !failed)} onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
      {src && !failed && <img key={`${src}:${reloadKey}`} className={styles.image} src={src} alt={alt} draggable={false} style={{ width: loaded ? natural.width * zoom : undefined, height: loaded ? natural.height * zoom : undefined, visibility: loaded ? 'visible' : 'hidden' }} onLoad={event => {
        const image = event.currentTarget;
        setImageFailed(false);
        setNatural({ width: image.naturalWidth, height: image.naturalHeight });
      }} onError={() => setImageFailed(true)} />}
      {!loaded && <div className={styles.status} role={failed ? 'alert' : 'status'}>
        <span>{error ?? (imageFailed
          ? t('common.imagePreview.loadFailed', { defaultValue: '图片加载失败' })
          : t('common.imagePreview.loading', { defaultValue: '正在加载图片…' }))}</span>
        {failed && onRetry ? <Button type='secondary' onClick={retry}>{t('common.retry')}</Button> : null}
      </div>}
    </div>
    {hasNavigation && navigation ? <>
      <div className={`${styles.navigation} ${styles.previous}`}><Tooltip content={previousLabel}><button type='button' aria-label={previousLabel} disabled={navigation.index <= 0} onClick={() => navigation.onIndexChange(navigation.index - 1)}><Left size={24} fill='currentColor' /></button></Tooltip></div>
      <div className={`${styles.navigation} ${styles.next}`}><Tooltip content={nextLabel}><button type='button' aria-label={nextLabel} disabled={navigation.index >= navigation.total - 1} onClick={() => navigation.onIndexChange(navigation.index + 1)}><Right size={24} fill='currentColor' /></button></Tooltip></div>
      <div className={styles.position} role='status' aria-live='polite' aria-atomic='true'>{t('common.imagePreview.position', { defaultValue: '{{current}} / {{total}}', current: navigation.index + 1, total: navigation.total })}</div>
    </> : null}
    <div className={styles.actions}>
      {onSaveAs ? <Tooltip content={saveAsLabel}><button type='button' aria-label={saveAsLabel} disabled={saving || !loaded} onClick={() => void saveAs()}><Download size={20} fill='currentColor' /></button></Tooltip> : null}
      <Tooltip content={t('common.close')}><button type='button' aria-label={closeLabel} onClick={onClose}><Close size={20} fill='currentColor' /></button></Tooltip>
    </div>
    <div className={styles.zoom} role='group' aria-label={t('common.imagePreview.zoom', { defaultValue: '图片缩放' })}>
      <Tooltip content={t('common.imagePreview.zoomOut', { defaultValue: '缩小图片' })}><button type='button' aria-label={t('common.imagePreview.zoomOut', { defaultValue: '缩小图片' })} disabled={!loaded || zoom <= .01} onClick={() => setScale(Math.max(.01, zoom / 1.25))}><Minus size={20} fill='currentColor' /></button></Tooltip>
      <Tooltip content={fitLabel}><button type='button' className={styles.percentage} aria-label={fitLabel} disabled={!loaded} onClick={() => setScale(null)}>{loaded ? `${Math.round(zoom * 100)}%` : '—'}</button></Tooltip>
      <Tooltip content={t('common.imagePreview.zoomIn', { defaultValue: '放大图片' })}><button type='button' aria-label={t('common.imagePreview.zoomIn', { defaultValue: '放大图片' })} disabled={!loaded || zoom >= 4} onClick={() => setScale(Math.min(4, zoom * 1.25))}><Plus size={20} fill='currentColor' /></button></Tooltip>
    </div>
  </Modal>;
}
