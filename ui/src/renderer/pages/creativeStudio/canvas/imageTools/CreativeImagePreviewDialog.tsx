/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import ImageLightbox from '@/renderer/components/media/ImageLightbox';
import {
  creativeAssetClient,
  isCreativeAssetDeleted,
  saveCreativeAssetAs,
  subscribeCreativeAssetDeletion,
  type CreativeAsset,
} from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import { CREATIVE_CANVAS_MODAL_Z_INDEX } from '../canvasOverlayLayers';

type ImageNode = Extract<CreativeCanvasNode, { type: 'image' }>;

interface CreativeImagePreviewDialogProps {
  node: ImageNode;
  resolveAsset(node: ImageNode): Promise<CreativeAsset>;
  onClose(): void;
}

/** Mounted for one preview session; never changes the canvas document or viewport. */
const CreativeImagePreviewDialog: React.FC<CreativeImagePreviewDialogProps> = ({
  node,
  resolveAsset,
  onClose,
}) => {
  const { t } = useTranslation();
  const returnFocusRef = useRef(
    typeof document === 'undefined' ? null : document.activeElement
  );
  const [asset, setAsset] = useState<CreativeAsset | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    const refresh = () => setAttempt((current) => current + 1);
    const unsubscribe = subscribeCreativeAssetDeletion(creativeAssetClient, (assetId) => {
      if (assetId === node.data.assetId) refresh();
    });
    window.addEventListener('focus', refresh);
    return () => { unsubscribe(); window.removeEventListener('focus', refresh); };
  }, [node.data.assetId]);

  useEffect(() => {
    let active = true;
    setAsset(null);
    setFailed(false);
    void resolveAsset(node).then(
      (resolved) => {
        if (!active) return;
        if (resolved.kind !== 'image' || (!isCreativeAssetDeleted(resolved) && !resolved.originalUrl.trim())) {
          setFailed(true);
          return;
        }
        setAsset(resolved);
      },
      () => {
        if (active) setFailed(true);
      }
    );
    return () => { active = false; };
  }, [attempt, node, resolveAsset]);

  useEffect(() => () => {
    const target = returnFocusRef.current;
    queueMicrotask(() => {
      if (target instanceof HTMLElement && target.isConnected) {
        target.focus({ preventScroll: true });
      }
    });
  }, []);

  const title = t('creativeStudio.canvas.imageTools.toolbar.previewLabel');
  const deleted = Boolean(asset && isCreativeAssetDeleted(asset));
  const error = deleted
    ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
    : failed
      ? t('creativeStudio.canvas.imageTools.preview.loadFailed')
      : null;

  return (
    <ImageLightbox
      key={`${node.id}:${attempt}`}
      src={asset && !deleted ? asset.originalUrl : null}
      title={title}
      alt={asset?.title || node.data.alt || node.data.caption || title}
      loading={!asset && !failed}
      error={error}
      onRetry={!deleted ? () => setAttempt((current) => current + 1) : undefined}
      onSaveAs={asset && !deleted
        ? () => saveCreativeAssetAs(asset, t('common.saveAs', { defaultValue: '另存为' }))
        : undefined}
      // Escape the canvas shell's viewport-portaled composers (1600) and
      // node toolbars (1601), while staying in the shared preview container.
      zIndex={CREATIVE_CANVAS_MODAL_Z_INDEX}
      onClose={onClose}
    />
  );
};

export default CreativeImagePreviewDialog;
