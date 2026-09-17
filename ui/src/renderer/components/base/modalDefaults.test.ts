import '../../../../test/setup-dom.ts';
import { expect, test } from 'bun:test';
import { modalDefaults } from './modalDefaults';

test('dialog menus escape the scrolling body while remaining inside its focus lock', () => {
  const dialog = document.createElement('div');
  dialog.className = 'arco-modal';
  dialog.innerHTML = '<div data-focus-lock-disabled="false"><div class="arco-modal-content"><button>Choose</button></div></div>';
  const trigger = dialog.querySelector('button')!;
  expect(modalDefaults.getChildrenPopupContainer!(trigger)).toBe(dialog.firstElementChild as HTMLElement);
  dialog.firstElementChild!.removeAttribute('data-focus-lock-disabled');
  expect(modalDefaults.getChildrenPopupContainer!(trigger)).toBe(dialog);
  expect(modalDefaults.getChildrenPopupContainer!(document.createElement('button'))).toBe(document.body);
});
