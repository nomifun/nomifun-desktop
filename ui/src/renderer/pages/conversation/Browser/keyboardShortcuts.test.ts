import { expect, test } from 'bun:test';
import { browserShortcut } from './keyboardShortcuts';

test('browser chrome recognizes only its supported shortcuts, without swallowing AltGr or IME input', () => {
  const base = { key:'',ctrlKey:false,altKey:false,metaKey:false,shiftKey:false,isComposing:false };
  for (const [key, action] of [['l','address'],['t','new_tab'],['w','close_tab'],['r','reload']] as const) {
    expect(browserShortcut({...base,key,ctrlKey:true})).toBe(action);
    expect(browserShortcut({...base,key,ctrlKey:true,altKey:true})).toBeNull();
    expect(browserShortcut({...base,key,ctrlKey:true,shiftKey:true})).toBeNull();
    expect(browserShortcut({...base,key,ctrlKey:true,isComposing:true})).toBeNull();
  }
  expect(browserShortcut({...base,key:'F5'})).toBe('reload');
  expect(browserShortcut({...base,key:'ArrowLeft',altKey:true})).toBe('back');
  expect(browserShortcut({...base,key:'ArrowRight',altKey:true})).toBe('forward');
  expect(browserShortcut({...base,key:'l',metaKey:true})).toBeNull();
  expect(browserShortcut({...base,key:'constructor',ctrlKey:true})).toBeNull();
});
