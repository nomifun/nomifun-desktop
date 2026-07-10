export interface RequirementTagSummary {
  tag: string;
  done: number;
  total: number;
}

export interface RequirementTagLoadState {
  tags: RequirementTagSummary[];
  loading: boolean;
  error: string | null;
}

export type RequirementTagLoadAction =
  | { type: 'start' }
  | { type: 'success'; tags: RequirementTagSummary[] }
  | { type: 'failure'; error: string }
  | { type: 'finish' };

export const initialRequirementTagLoadState: RequirementTagLoadState = {
  tags: [],
  loading: false,
  error: null,
};

export function reduceRequirementTagLoadState(
  state: RequirementTagLoadState,
  action: RequirementTagLoadAction
): RequirementTagLoadState {
  switch (action.type) {
    case 'start':
      return { ...state, loading: true };
    case 'success':
      return { ...state, tags: action.tags, error: null };
    case 'failure':
      return { ...state, error: action.error };
    case 'finish':
      return { ...state, loading: false };
  }
}
