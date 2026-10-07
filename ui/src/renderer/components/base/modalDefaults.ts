import type { ModalProps } from '@arco-design/web-react';

/** Keep menus outside the scrolling body, but inside the dialog's focus lock. */
export const modalDefaults: Partial<ModalProps> = {
  alignCenter: true,
  getChildrenPopupContainer: node =>
    node.closest<HTMLElement>('[data-focus-lock-disabled]') ??
    node.closest<HTMLElement>('.arco-modal') ?? document.body,
};
