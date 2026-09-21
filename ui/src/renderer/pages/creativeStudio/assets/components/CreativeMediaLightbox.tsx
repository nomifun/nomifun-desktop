/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Close, Download } from '@icon-park/react';
import { Modal, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import ImageLightbox from '@/renderer/components/media/ImageLightbox';
import imageLightboxStyles from '@/renderer/components/media/ImageLightbox.module.css';
import CreativeVideoPlayer from './CreativeVideoPlayer';
import styles from './CreativeMediaLightbox.module.css';

interface CreativeMediaLightboxProps {
  kind: 'image' | 'video';
  src: string;
  posterSrc?: string | null;
  title: string;
  onClose(): void;
  zIndex?: number;
}

/** Full-resolution preview shared by compact creative-media references. */
const CreativeMediaLightbox: React.FC<CreativeMediaLightboxProps> = ({
  kind,
  src,
  posterSrc,
  title,
  onClose,
  zIndex,
}) => {
  const { t } = useTranslation();
  if (kind === 'image') {
    return <ImageLightbox src={src} title={title} onClose={onClose} zIndex={zIndex} />;
  }

  const downloadLabel = t('common.download', { defaultValue: '下载' });
  const closeLabel = t('common.close', { defaultValue: '关闭' });
  const closePreviewLabel = t('creativeStudio.canvas.video.closePreview', {
    defaultValue: '关闭视频预览',
  });
  return (
    <Modal
      visible
      title={title}
      className={`nomifun-modal-fullscreen ${imageLightboxStyles.modal}`}
      wrapClassName={imageLightboxStyles.wrap}
      maskStyle={{ background: 'rgba(0, 0, 0, .88)', zIndex }}
      wrapStyle={zIndex === undefined ? undefined : { zIndex }}
      footer={null}
      closable={false}
      focusLock
      unmountOnExit
      onCancel={onClose}
    >
      <div className={`${imageLightboxStyles.viewport} ${styles.videoViewport}`}>
        <div className={styles.videoFrame}>
          <CreativeVideoPlayer
            src={src}
            poster={posterSrc ?? undefined}
            label={title}
          />
        </div>
      </div>
      <div className={imageLightboxStyles.actions}>
        <Tooltip content={downloadLabel}>
          <a href={src} download={title} aria-label={`${downloadLabel}：${title}`}>
            <Download size={20} fill='currentColor' />
          </a>
        </Tooltip>
        <Tooltip content={closeLabel}>
          <button type='button' aria-label={closePreviewLabel} onClick={onClose}>
            <Close size={20} fill='currentColor' />
          </button>
        </Tooltip>
      </div>
    </Modal>
  );
};

export default CreativeMediaLightbox;
