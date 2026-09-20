import { useEffect, useState } from 'react';
import { Message, Modal, Tooltip } from '@arco-design/web-react';
import { Close, Download, Minus, Plus } from '@icon-park/react';
import styles from './ImageLightbox.module.css';

export default function ImageLightbox({ src, title, onClose, onDownload, zIndex }: {
  src: string;
  title: string;
  onClose(): void;
  onDownload?: () => Promise<void>;
  /** Raise the lightbox above product-local overlay stacks such as the canvas composer. */
  zIndex?: number;
}) {
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  const [bounds, setBounds] = useState({ width: 1, height: 1 });
  const [natural, setNatural] = useState({ width: 0, height: 0 });
  const [scale, setScale] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  const [downloading, setDownloading] = useState(false);
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
  const loaded = natural.width > 0 && !failed;
  const download = async () => {
    if (!onDownload || downloading) return;
    setDownloading(true);
    try { await onDownload(); }
    catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setDownloading(false); }
  };
  return <Modal visible title='查看图片' className={`nomifun-modal-fullscreen ${styles.modal}`} wrapClassName={styles.wrap} maskStyle={{ background: 'rgba(0, 0, 0, .88)', zIndex }} wrapStyle={zIndex === undefined ? undefined : { zIndex }} footer={null} closable={false} focusLock unmountOnExit onCancel={onClose}>
    <div className={styles.viewport} ref={setViewport} onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
      {!failed && <img className={styles.image} src={src} alt={title} draggable={false} style={{ width: loaded ? natural.width * zoom : undefined, height: loaded ? natural.height * zoom : undefined, visibility: loaded ? 'visible' : 'hidden' }} onLoad={event => {
        const image = event.currentTarget;
        setNatural({ width: image.naturalWidth, height: image.naturalHeight });
      }} onError={() => setFailed(true)} />}
      {!loaded && <span className={styles.status} role='status'>{failed ? '图片加载失败' : '正在加载图片…'}</span>}
    </div>
    <div className={styles.actions}>
      {onDownload ? <Tooltip content='下载图片'><button type='button' aria-label='下载图片' disabled={downloading || !loaded} onClick={() => void download()}><Download size={20} fill='currentColor' /></button></Tooltip>
        : <Tooltip content='下载图片'><a href={src} download={title} aria-label='下载图片'><Download size={20} fill='currentColor' /></a></Tooltip>}
      <Tooltip content='关闭'><button type='button' aria-label='关闭图片预览' onClick={onClose}><Close size={20} fill='currentColor' /></button></Tooltip>
    </div>
    <div className={styles.zoom} role='group' aria-label='图片缩放'>
      <Tooltip content='缩小'><button type='button' aria-label='缩小图片' disabled={!loaded || zoom <= .01} onClick={() => setScale(Math.max(.01, zoom / 1.25))}><Minus size={20} fill='currentColor' /></button></Tooltip>
      <Tooltip content='适应窗口'><button type='button' className={styles.percentage} aria-label='适应窗口' disabled={!loaded} onClick={() => setScale(null)}>{loaded ? `${Math.round(zoom * 100)}%` : '—'}</button></Tooltip>
      <Tooltip content='放大'><button type='button' aria-label='放大图片' disabled={!loaded || zoom >= 4} onClick={() => setScale(Math.min(4, zoom * 1.25))}><Plus size={20} fill='currentColor' /></button></Tooltip>
    </div>
  </Modal>;
}
