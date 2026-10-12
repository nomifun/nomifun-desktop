import { expect, test } from 'bun:test';
import { formatProjectWorkpathDisplay, withAppProjectNames } from './appProjectWorkpaths';
import { formatWorkpathDisplay } from './sidebarDisplayPreferences';
import { buildWorkpathTree } from './workpathTree';

test('same-name applications keep separate path groups and the existing grouping and ordering rules', () => {
  const paths = ['C:/apps/app-one', 'C:/apps/app-two'];
  const tree = buildWorkpathTree([], [], [paths[1]], paths);
  const labeled = withAppProjectNames(tree, paths.map(directory => ({ directory, name: 'Todo' })));
  expect(labeled.map(node => node.key)).toEqual(tree.map(node => node.key));
  expect(labeled.filter(node => node.applicationName === 'Todo')).toHaveLength(2);
  expect(labeled[0].key).toBe(paths[1]); expect(labeled[0].pinned).toBe(true);
  for (let index = 0; index < tree.length; index++) {
    expect(labeled[index].interactive).toBe(tree[index].interactive);
    expect(labeled[index].terminal).toBe(tree[index].terminal);
    expect(tree[index].applicationName).toBeUndefined();
  }
});

test('owner names use the existing exact workpath normalization, with no case or parent-folder merging', () => {
  const paths = ['C:/apps/project', 'C:/apps/project/child', 'C:/apps/Project'];
  const tree = buildWorkpathTree([], [], [], paths);
  const labeled = withAppProjectNames(tree, [{ directory: ' C:\\apps\\project\\ ', name: 'Calendar' }]);
  expect(labeled.find(node => node.key === paths[0])?.displayName).toBe('Calendar');
  expect(labeled.find(node => node.key === paths[1])?.applicationName).toBeUndefined();
  expect(labeled.find(node => node.key === paths[2])?.applicationName).toBeUndefined();
  expect(labeled.map(node => node.key)).toEqual(tree.map(node => node.key));
});

test('application names remain readable for every display mode while full paths stay available', () => {
  const tree = buildWorkpathTree([], [], [], ['C:/apps/owned-app']);
  const node = withAppProjectNames(tree, [{ directory: 'C:/apps/owned-app', name: 'Calendar' }]).find(node => node.applicationName)!;
  for (const mode of ['compressed', 'folder', 'full', 'folderWithPath'] as const) {
    const display = formatProjectWorkpathDisplay(node, mode);
    expect(display.kind).not.toBe('compressed');
    expect('primary' in display && display.primary).toBe('Calendar');
    expect(display.tooltip).toBe(node.key);
    if (mode === 'full' || mode === 'folderWithPath') expect('secondary' in display && display.secondary).toBe(node.key);
  }
  const ordinary = tree.find(node => node.key === 'C:/apps/owned-app')!;
  expect(formatProjectWorkpathDisplay(ordinary, 'compressed')).toEqual(formatWorkpathDisplay(ordinary.key, ordinary.displayName, 'compressed'));
  expect(withAppProjectNames(tree, undefined)).toBe(tree);
});
