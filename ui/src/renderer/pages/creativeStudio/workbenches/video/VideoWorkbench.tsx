/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import VideoWorkbenchComposer from './VideoWorkbenchComposer';
import VideoWorkbenchResults from './VideoWorkbenchResults';
import { videoResultsState } from './presentation';
import styles from './VideoWorkbench.module.css';
import type { VideoWorkbenchProps } from './types';

/**
 * Controlled visual workbench. Model lookup, assets, generation, persistence,
 * downloads and deletion remain owned by adapters outside this directory.
 */
const VideoWorkbench: React.FC<VideoWorkbenchProps> = (props) => {
  const className = props.className
    ? `${styles.workbench} ${props.className}`
    : styles.workbench;

  return (
    <div
      className={className}
      data-video-workbench
      data-workbench-layout={props.layout}
      data-results-state={videoResultsState(props.tasks)}
    >
      <main className={props.layout === 'side' ? styles.sideLayout : styles.bottomLayout}>
        {props.layout === 'side' ? <VideoWorkbenchComposer {...props} /> : null}
        <VideoWorkbenchResults {...props} />
        {props.layout === 'bottom' ? <VideoWorkbenchComposer {...props} /> : null}
      </main>
    </div>
  );
};

export default VideoWorkbench;
