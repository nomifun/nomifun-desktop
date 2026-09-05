/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import CreativeVideoPlayer from '../../assets/components/CreativeVideoPlayer';
import type { CreativeNodeAssetPresentation, CreativeNodeOfKind } from './types';

export { formatCreativeVideoTime as formatVideoNodeTime } from '../../assets/components/CreativeVideoPlayer';

interface CreativeVideoNodeMediaProps {
  node: CreativeNodeOfKind<'video'>;
  asset: CreativeNodeAssetPresentation;
  title: string;
  selected?: boolean;
  onActivate?: () => void;
}

/** Canvas selection is independent of the shared player's transient media state. */
const CreativeVideoNodeMedia: React.FC<CreativeVideoNodeMediaProps> = ({
  node, asset, title, selected, onActivate,
}) => (
  <CreativeVideoPlayer
    src={asset.src}
    poster={asset.posterSrc}
    label={asset.alt ?? asset.label ?? title}
    variant='canvas'
    autoPlay={node.data.autoplay}
    loop={node.data.loop}
    muted={node.data.muted}
    onPlayRequest={() => { if (!selected) onActivate?.(); }}
  />
);

export default CreativeVideoNodeMedia;
