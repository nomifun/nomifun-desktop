import { afterEach, expect, spyOn, test } from 'bun:test';
import { addRecentLaunchCommand, getRecentLaunchCommands, RECENT_LAUNCH_COMMANDS_KEY } from './recentLaunchCommands';

const restore: Array<() => void> = [];
afterEach(() => { restore.splice(0).reverse().forEach(dispose => dispose()); });
function storage() {
  const values = new Map<string, string>();
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const isolated = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
  };
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: isolated });
  restore.push(() => {
    if (previous) Object.defineProperty(globalThis, 'localStorage', previous);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  });
  const get = spyOn(isolated, 'getItem');
  const set = spyOn(isolated, 'setItem');
  restore.push(() => get.mockRestore(), () => set.mockRestore());
  return { values, get, set };
}

test('recent commands trim edges, retain command content, dedupe and cap the MRU list at five', () => {
  storage();
  for (const command of ['one', 'two', 'three', 'four', 'five', 'six', '  three  ', 'echo "a  b"', '   ']) {
    addRecentLaunchCommand(command);
  }
  expect(getRecentLaunchCommands()).toEqual(['echo "a  b"', 'three', 'six', 'five', 'four']);
});

test('bad stored JSON or non-array data is empty; mixed arrays retain only strings', () => {
  const { values } = storage();
  for (const raw of ['{', 'null', '{}']) {
    values.set(RECENT_LAUNCH_COMMANDS_KEY, raw);
    expect(getRecentLaunchCommands()).toEqual([]);
  }
  values.set(RECENT_LAUNCH_COMMANDS_KEY, JSON.stringify(['one', 2, null, 'two']));
  expect(getRecentLaunchCommands()).toEqual(['one', 'two']);
});

test('storage access and quota errors do not break the launch path', () => {
  const { get, set } = storage();
  get.mockImplementation(() => { throw new Error('blocked'); });
  expect(getRecentLaunchCommands()).toEqual([]);
  set.mockImplementation(() => { throw new Error('quota'); });
  expect(() => addRecentLaunchCommand('shell')).not.toThrow();
});
