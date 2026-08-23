/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import {
  forwardRef,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
} from 'react';
import { useTranslation } from 'react-i18next';

import type { DirectorState } from '../domain';
import styles from './DirectorRuntimeViewport.module.css';
import { ThreeDirectorRuntime } from './ThreeDirectorRuntime';
import type {
  DirectorAssetUrlResolver,
  DirectorRuntimeError,
  DirectorRuntimeHandle,
} from './types';

export interface DirectorRuntimeViewportProps {
  state: DirectorState;
  resolveAssetUrl: DirectorAssetUrlResolver;
  timeSeconds?: number;
  maxPixelRatio?: number;
  showAxes?: boolean;
  className?: string;
  ariaLabel?: string;
  onError?(error: DirectorRuntimeError): void;
}

/**
 * React host for the imperative renderer. It is designed to be passed directly
 * to DirectorWorkbenchShell's `viewportSlot`; the controller owns project state
 * and receives captures through the forwarded runtime handle.
 */
export const DirectorRuntimeViewport = forwardRef<
  DirectorRuntimeHandle,
  DirectorRuntimeViewportProps
>(function DirectorRuntimeViewport(
  {
    state,
    resolveAssetUrl,
    timeSeconds,
    maxPixelRatio,
    showAxes,
    className,
    ariaLabel,
    onError,
  },
  ref
) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const runtimeRef = useRef<ThreeDirectorRuntime | null>(null);
  const resolverRef = useRef(resolveAssetUrl);
  const errorRef = useRef(onError);
  resolverRef.current = resolveAssetUrl;
  errorRef.current = onError;

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    try {
      const runtime = new ThreeDirectorRuntime({
        container,
        resolveAssetUrl: (assetId, signal) => resolverRef.current(assetId, signal),
        maxPixelRatio,
        showAxes,
        onError: (error) => errorRef.current?.(error),
      });
      runtimeRef.current = runtime;
    } catch (cause) {
      errorRef.current?.({
        code: 'renderer',
        message: 'Unable to initialize the Three.js Director renderer',
        cause,
      });
    }
    return () => {
      runtimeRef.current?.dispose();
      runtimeRef.current = null;
    };
  }, [maxPixelRatio, showAxes]);

  useLayoutEffect(() => {
    runtimeRef.current?.update(state, timeSeconds);
  }, [state, timeSeconds]);

  useImperativeHandle(
    ref,
    () => ({
      get canvas() {
        const runtime = runtimeRef.current;
        if (!runtime) throw new Error('Director runtime is not mounted');
        return runtime.canvas;
      },
      update(nextState, nextTime) {
        runtimeRef.current?.update(nextState, nextTime);
      },
      resize() {
        runtimeRef.current?.resize();
      },
      start() {
        runtimeRef.current?.start();
      },
      stop() {
        runtimeRef.current?.stop();
      },
      captureImage(request) {
        const runtime = runtimeRef.current;
        if (!runtime) return Promise.reject(new Error('Director runtime is not mounted'));
        return runtime.captureImage(request);
      },
      dispose() {
        runtimeRef.current?.dispose();
        runtimeRef.current = null;
      },
    }),
    []
  );

  return (
    <div
      ref={containerRef}
      className={classNames(styles.viewport, className)}
      role='application'
      aria-label={
        ariaLabel ??
        t('creativeStudio.director.runtime.viewportLabel', {
          defaultValue: 'Three.js 3D导演视口',
        })
      }
      tabIndex={0}
      data-director-runtime-viewport
    />
  );
});

export default DirectorRuntimeViewport;
