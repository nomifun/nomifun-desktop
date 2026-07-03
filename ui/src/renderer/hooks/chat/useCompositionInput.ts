import { useCallback, useRef, useState } from 'react';

export type SendKeyMode = 'enter' | 'mod-enter';

type ComposingState = { composing: boolean; justComposed: boolean };
type ImeKeyLike = { key?: string; keyCode?: number; nativeEvent?: { isComposing?: boolean } };
type SubmitKeyLike = { key: string; shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean };
type TextEditingShortcutLike = { key: string; shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean };

const SYSTEM_TEXT_EDITING_KEYS = new Set(['a', 'c', 'v', 'x', 'y', 'z']);

/**
 * 纯函数：判断这次 keydown 是否处于输入法合成中（绝不能触发发送）。
 * 组合多重信号以覆盖各浏览器/输入法的事件时序差异：
 * - composing：compositionstart→true / compositionend→false 的 ref
 * - justComposed：compositionend 后的一帧兜底窗口（覆盖"compositionend 先于 Enter keydown"）
 * - nativeEvent.isComposing：W3C 原生属性
 * - keyCode === 229：IME 处理中的 keydown
 */
export function isImeComposingKey(e: ImeKeyLike, state: ComposingState): boolean {
  return state.composing || state.justComposed || e.nativeEvent?.isComposing === true || e.keyCode === 229;
}

/**
 * 纯函数：在给定发送键偏好下，这次 keydown 是否为"提交"手势。
 * - 'enter'（默认，兼容旧行为）：Enter 且非 Shift 即提交（Cmd/Ctrl+Enter 也提交）
 * - 'mod-enter'：必须 Cmd/Ctrl+Enter；裸 Enter 不提交（留给 textarea 换行）
 */
export function isSubmitGesture(e: SubmitKeyLike, mode: SendKeyMode): boolean {
  if (e.key !== 'Enter' || e.shiftKey) return false;
  if (mode === 'mod-enter') return Boolean(e.metaKey || e.ctrlKey);
  return true;
}

/**
 * 纯函数：判断这次 keydown 是否应交还给 textarea 的原生编辑行为。
 * 复制/粘贴/剪切/全选/撤销/重做必须优先于浮层导航和发送拦截，否则终端会话
 * 底部输入框等 SendBox 复用场景会吞掉用户熟悉的系统快捷键。
 */
export function isSystemTextEditingShortcut(e: TextEditingShortcutLike): boolean {
  if (e.key === 'Insert') {
    return !e.metaKey && !e.altKey && ((Boolean(e.ctrlKey) && !e.shiftKey) || (Boolean(e.shiftKey) && !e.ctrlKey));
  }

  if (e.altKey || (!e.metaKey && !e.ctrlKey)) {
    return false;
  }

  return SYSTEM_TEXT_EDITING_KEYS.has(e.key.toLowerCase());
}

/**
 * 共享的输入法合成事件处理hook
 * 消除SendBox组件和GUID页面中的IME处理重复代码
 */
export const useCompositionInput = () => {
  const isComposing = useRef(false);
  const justComposedRef = useRef(false);
  const [isComposingState, setIsComposingState] = useState(false);

  const compositionHandlers = {
    onCompositionStartCapture: () => {
      isComposing.current = true;
      justComposedRef.current = false;
      setIsComposingState(true);
    },
    onCompositionEndCapture: () => {
      isComposing.current = false;
      setIsComposingState(false);
      // 一帧兜底：覆盖 compositionend 同 tick 先于 Enter keydown 的浏览器，
      // 同时保证之后用户主动再按 Enter 仍能正常发送。
      justComposedRef.current = true;
      requestAnimationFrame(() => {
        justComposedRef.current = false;
      });
    },
  };

  const isImeActive = useCallback(
    (e: ImeKeyLike) => isImeComposingKey(e, { composing: isComposing.current, justComposed: justComposedRef.current }),
    []
  );

  const createKeyDownHandler = (
    onEnterPress: () => void,
    onKeyDownIntercept?: (e: React.KeyboardEvent) => boolean,
    sendKey: SendKeyMode = 'enter'
  ) => {
    return (e: React.KeyboardEvent) => {
      if (isImeActive(e)) return;
      if (isSystemTextEditingShortcut(e)) return;
      if (onKeyDownIntercept?.(e)) return;
      if (isSubmitGesture(e, sendKey)) {
        e.preventDefault();
        onEnterPress();
      }
    };
  };

  return {
    isComposing,
    isComposingState,
    compositionHandlers,
    createKeyDownHandler,
    isImeActive,
  };
};
