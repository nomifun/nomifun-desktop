import type { BrowserShortcutAction } from './client';

export function browserShortcut(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey' | 'isComposing'>): BrowserShortcutAction | null {
  if (event.isComposing || event.shiftKey || ((event.ctrlKey || event.metaKey) && event.altKey)) return null;
  if (event.ctrlKey || event.metaKey) {
    switch (event.key.toLowerCase()) {
      case 'l': return 'address';
      case 't': return 'new_tab';
      case 'w': return 'close_tab';
      case 'r': return 'reload';
      default: return null;
    }
  }
  if (event.altKey) return event.key === 'ArrowLeft' ? 'back' : event.key === 'ArrowRight' ? 'forward' : null;
  return event.key === 'F5' ? 'reload' : null;
}
