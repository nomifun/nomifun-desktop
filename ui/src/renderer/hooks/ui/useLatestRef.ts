/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { useRef, useLayoutEffect } from 'react';

/**
 * 保持值的最新引用，避免闭包陷阱
 * Keep the latest reference of a value to avoid closure trap
 *
 * @example
 * ```tsx
 * const setContentRef = useLatestRef(setContent);
 * useEffect(() => {
 *   const handler = (text: string) => {
 *     setContentRef.current(text);
 *   };
 *   // ...
 * }, []); // 依赖数组为空，不会因为 setContent 变化而重新注册
 * ```
 *
 * @param value - 需要保持最新引用的值 / The value to keep latest reference
 * @returns 包含最新值的 ref 对象 / A ref object containing the latest value
 */
export function useLatestRef<T>(value: T) {
  const ref = useRef(value);

  // 使用 useLayoutEffect 确保在渲染完成前同步更新
  // Use useLayoutEffect to ensure synchronous update before render completes
  useLayoutEffect(() => {
    ref.current = value;
  });

  return ref;
}
