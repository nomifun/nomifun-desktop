import { describe, expect, test } from 'bun:test';
import { absoluteToRelativePath, fromBackendWorkspaceList } from './workspaceMapper';

describe('conversation workspace root contract', () => {
  test('the root request and returned file use the same relative namespace', () => {
    const workspace = 'C:/Projects/test';
    const root = absoluteToRelativePath(workspace, workspace);
    expect(root).toBe('.');

    const tree = fromBackendWorkspaceList([{ name: 'snake_game.html', type: 'file' }], workspace, root);
    expect(tree).toHaveLength(1);
    expect(tree[0].children?.[0]).toMatchObject({
      name: 'snake_game.html',
      relativePath: 'snake_game.html',
      fullPath: 'C:/Projects/test/snake_game.html',
      isFile: true,
    });
  });
});
