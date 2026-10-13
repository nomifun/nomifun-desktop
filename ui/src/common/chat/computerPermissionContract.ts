/** Identities and native operations from nomi-computer/src/capability.rs. */
export const COMPUTER_OBSERVE_ACTION_ID = 'computer/observe';
export const COMPUTER_A11Y_OBSERVE_ACTION_ID = 'computer/a11y.observe';
export const COMPUTER_INPUT_ACTION_ID = 'computer/input';
const INPUT_OPERATIONS = new Set([
  'click_element', 'set_element_value', 'right_click_element', 'double_click_element',
  'left_click', 'right_click', 'middle_click', 'double_click', 'triple_click', 'mouse_move',
  'left_click_drag', 'type', 'key', 'scroll', 'focus_window',
]);

export const computerPermissionKindsForOperation = (
  actionId: unknown,
  operation: unknown
): Array<'accessibility' | 'screen_recording'> => {
  if (actionId === COMPUTER_OBSERVE_ACTION_ID && operation === 'screenshot') return ['screen_recording'];
  if (actionId === COMPUTER_A11Y_OBSERVE_ACTION_ID && operation === 'observe') return ['accessibility'];
  if (actionId === COMPUTER_INPUT_ACTION_ID && typeof operation === 'string' && INPUT_OPERATIONS.has(operation)) return ['accessibility'];
  return [];
};
