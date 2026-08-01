/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { useEffect, useRef, useState, type RefObject } from 'react';

interface UseContainerWidthOptions {
  /**
   * Start from (and fall back to) `window.innerWidth` while the element is
   * not mounted, instead of 0. Opt-in for layouts whose breakpoint math must
   * not see a transient 0-width "compact" frame before the first measure.
   */
  fallbackToWindowWidth?: boolean;
}

/**
 * 观测某个容器元素的实际宽度（基于 ResizeObserver）。
 *
 * 用于做“以内容容器宽度为准”的响应式判断，而不是用视口宽度——
 * 因为侧边栏等会占用宽度，视口宽度并不等于内容可用宽度。
 *
 * @example
 * const { ref, width } = useContainerWidth<HTMLDivElement>();
 * return <div ref={ref}>{width < 600 ? <Compact/> : <Full/>}</div>;
 */
export function useContainerWidth<T extends HTMLElement = HTMLDivElement>(
  options?: UseContainerWidthOptions
): {
  ref: RefObject<T | null>;
  width: number;
} {
  const fallbackToWindowWidth = options?.fallbackToWindowWidth ?? false;
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(() =>
    fallbackToWindowWidth && typeof window !== 'undefined' ? window.innerWidth : 0
  );

  useEffect(() => {
    const el = ref.current;
    if (!el) {
      if (fallbackToWindowWidth) {
        setWidth(typeof window === 'undefined' ? 0 : window.innerWidth);
      }
      return;
    }

    const update = () => setWidth(el.getBoundingClientRect().width);
    update();

    const observer = new ResizeObserver(update);
    observer.observe(el);
    window.addEventListener('resize', update);

    return () => {
      observer.disconnect();
      window.removeEventListener('resize', update);
    };
  }, [fallbackToWindowWidth]);

  return { ref, width };
}

export default useContainerWidth;
