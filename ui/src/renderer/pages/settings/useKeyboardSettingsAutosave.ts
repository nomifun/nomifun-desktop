import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { keyboard, type KeyboardState } from '@/common/keyboard';
import {
  createKeyboardSettingsAutosaveController,
  type KeyboardSettingsAutosaveController,
} from './keyboardSettingsAutosave';

const controllers = new WeakMap<typeof keyboard.updateSettings, KeyboardSettingsAutosaveController>();

/** One in-memory writer survives page navigation; mounted pages reuse useKeyboard's state source. */
export function useKeyboardSettingsAutosave(
  state: KeyboardState | null,
  { captureBusy = false, debounceMs = 400 }: { captureBusy?: boolean; debounceMs?: number } = {},
) {
  const writer = keyboard.updateSettings;
  const controller = useMemo(() => {
    const existing = controllers.get(writer);
    if (existing) return existing;
    const created = createKeyboardSettingsAutosaveController({
      updateSettings: writer,
      getState: keyboard.getState,
      listen: keyboard.listen,
    }, { debounceMs });
    controllers.set(writer, created);
    return created;
  }, [writer]);
  const snapshot = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);

  // Attach first so the freshly mounted page supplies a current snapshot before queued work resumes.
  useEffect(() => controller.attach(), [controller]);
  useEffect(() => { controller.setDebounceMs(debounceMs); }, [controller, debounceMs]);
  useEffect(() => { controller.observe(state, captureBusy); }, [controller, state, captureBusy]);

  return {
    ...snapshot,
    change: controller.change,
    flush: controller.flush,
    retry: controller.retry,
    compositionStart: controller.compositionStart,
    compositionEnd: controller.compositionEnd,
  };
}
