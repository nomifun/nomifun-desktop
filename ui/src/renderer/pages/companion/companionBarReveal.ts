/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export interface CompanionBarRevealController {
  handleHoverChange: (over: boolean) => void;
  dispose: () => void;
}

export interface CompanionBarRevealControllerOptions<TTimer = ReturnType<typeof setTimeout>> {
  hideDelayMs: number;
  setRevealed: (next: boolean) => void;
  setTimeoutFn?: (fn: () => void, ms: number) => TTimer;
  clearTimeoutFn?: (handle: TTimer) => void;
}

/**
 * Keeps the mini chatbar latched briefly after the hit-test reports "outside".
 *
 * The desktop companion uses alpha-based click-through for transparent figure
 * pixels. Moving from a cutout figure to the chatbar can cross a tiny transparent
 * gap; hiding the chatbar immediately makes the still-visible input/expand
 * controls lose their hit target before the user can click them.
 */
export function createCompanionBarRevealController<TTimer = ReturnType<typeof setTimeout>>(
  opts: CompanionBarRevealControllerOptions<TTimer>
): CompanionBarRevealController {
  const setTimeoutFn =
    opts.setTimeoutFn ?? ((fn: () => void, ms: number) => setTimeout(fn, ms) as unknown as TTimer);
  const clearTimeoutFn =
    opts.clearTimeoutFn ?? ((handle: TTimer) => clearTimeout(handle as ReturnType<typeof setTimeout>));

  let revealed = false;
  let hideTimer: TTimer | null = null;

  const clearHideTimer = () => {
    if (hideTimer == null) return;
    clearTimeoutFn(hideTimer);
    hideTimer = null;
  };

  const setVisible = (next: boolean) => {
    if (revealed === next) return;
    revealed = next;
    opts.setRevealed(next);
  };

  return {
    handleHoverChange(over) {
      if (over) {
        clearHideTimer();
        setVisible(true);
        return;
      }
      if (hideTimer != null) return;
      hideTimer = setTimeoutFn(() => {
        hideTimer = null;
        setVisible(false);
      }, opts.hideDelayMs);
    },
    dispose() {
      clearHideTimer();
    },
  };
}
