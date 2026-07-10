import { describe, expect, test } from 'bun:test';
import { initialRequirementTagLoadState, reduceRequirementTagLoadState } from './requirementTagLoadState';

describe('requirement tag load state', () => {
  test('starts loading without discarding existing tags', () => {
    const current = {
      tags: [{ tag: 'release', done: 1, total: 2 }],
      loading: false,
      error: 'old error',
    };

    expect(reduceRequirementTagLoadState(current, { type: 'start' })).toEqual({
      tags: current.tags,
      loading: true,
      error: 'old error',
    });
  });

  test('stores successful tags and clears the previous error', () => {
    const tags = [{ tag: 'release', done: 2, total: 2 }];
    const current = { ...initialRequirementTagLoadState, loading: true, error: 'network' };

    expect(reduceRequirementTagLoadState(current, { type: 'success', tags })).toEqual({
      tags,
      loading: true,
      error: null,
    });
  });

  test('records failure while preserving the last successful tags', () => {
    const tags = [{ tag: 'release', done: 1, total: 2 }];
    const current = { tags, loading: true, error: null };

    expect(reduceRequirementTagLoadState(current, { type: 'failure', error: 'offline' })).toEqual({
      tags,
      loading: true,
      error: 'offline',
    });
  });

  test('finishes every request without changing data or error', () => {
    const current = { tags: [], loading: true, error: 'offline' };
    expect(reduceRequirementTagLoadState(current, { type: 'finish' })).toEqual({
      tags: [],
      loading: false,
      error: 'offline',
    });
  });
});
