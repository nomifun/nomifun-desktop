import { expect, test } from 'bun:test';
import { calculatorActivationReceipt, calculatorClearButtonRef, calculatorDisplayHasExactAdditionToken, calculatorDisplayHasExactValue, normalizeCalculatorBidi, sanitizeDesktopReport } from './global-os-desktop-cases.mjs';

test('Calculator display accepts exact numeric values with actual Bidi controls or actual RustDebug escapes', () => {
  for (const expected of ['0', '1', '46']) {
    for (const value of [expected, `\u200e${expected}\u200e`, `\u061c\u202a${expected}\u202c\u2066\u2069`,
      String.raw`\u{200e}${expected}\u{200e}`, String.raw`\u{61c}\u{202a}${expected}\u{202c}\u{2066}\u{2069}`]) {
      expect(calculatorDisplayHasExactValue(`[7] statictext "${value}"\n`, expected)).toBe(true);
      expect(calculatorDisplayHasExactValue(`[7] statictext = "${value}"\n`, expected)).toBe(true);
      expect(calculatorDisplayHasExactValue(`[7] textfield "Display" = "${value}" [focused]\n`, expected)).toBe(true);
    }
  }
});

test('Calculator display rejects other digits, expressions, button values and numeric labels contradicted by AXValue', () => {
  for (const [text, expected] of [
    ['[7] statictext "11"', '1'], ['[7] statictext "146"', '46'], ['[7] statictext "46.0"', '46'],
    ['[7] statictext "15"', '0'], ['[7] statictext "151"', '1'], ['[7] button "0"', '0'],
    ['[7] statictext "90÷6"', '6'], ['[7] statictext "1 + 46"', '46'], ['[7] statictext "46 "', '46'],
    ['[7] button "1"', '1'], ['[7] button "Display" = "46"', '46'], ['[7] radiobutton "46"', '46'],
    ['[7] statictext "1" = "11"', '1'], ['[7] statictext "46" = "146"', '46'],
    [`[7] statictext "\u200e146\u200e"`, '46'], [String.raw`[7] statictext "\u{200e}11\u{200e}"`, '1'],
    ['[7] statictext "\\u{200b}1"', '1'], [`[7] statictext "\u200b1"`, '1'],
    ['[7] statictext "\\n1"', '1'], [String.raw`[7] statictext "\\u{200e}1"`, '1'],
    [`[\u200e7] statictext "1"`, '1'], ['[7] button\\u{200e} "1"', '1'],
  ]) expect(calculatorDisplayHasExactValue(text, expected)).toBe(false);
});

test('Bidi normalization does not alter non-Bidi formatting or expand accepted numeric syntax', () => {
  expect(normalizeCalculatorBidi('"\u200e46\u200f"')).toBe('"46"');
  expect(normalizeCalculatorBidi(String.raw`"\u{200e}46\u{200f}"`)).toBe('"46"');
  for (const unchanged of ['" 46 "', '"46.0"', '"90÷6"', '"\u200b46"', String.raw`"\u{200b}46"`]) {
    expect(normalizeCalculatorBidi(unchanged)).toBe(unchanged);
  }
  for (const expected of ['1|11', '01', '', ' 46', '\u200e46']) {
    expect(calculatorDisplayHasExactValue('[7] statictext "46"', expected)).toBe(false);
  }
});

test('Modern Calculator exact current addition tokens never accept appended digits, buttons or replace final 46', () => {
  for (const token of ['12+', '12+3', '12+34']) {
    for (const value of [token, `\u200e${token}\u200e`, String.raw`\u{200e}${token}\u{200e}`]) {
      expect(calculatorDisplayHasExactAdditionToken(`[7] statictext = "${value}"`, token)).toBe(true);
      expect(calculatorDisplayHasExactValue(`[7] statictext = "${value}"`, '46')).toBe(false);
      expect(calculatorDisplayHasExactAdditionToken(`[7] button "${value}"`, token)).toBe(false);
    }
  }
  for (const value of ['123', '1234', '112+3', '12+34', '12 + 3', '12+3 ', '12+3=15', '3', '146']) {
    expect(calculatorDisplayHasExactAdditionToken(`[7] statictext = "${value}"`, '12+3')).toBe(false);
  }
  expect(calculatorDisplayHasExactAdditionToken('[7] statictext = "46"', '46')).toBe(false);
  expect(calculatorDisplayHasExactValue('[7] statictext = "12+34=46"', '46')).toBe(false);
  expect(calculatorDisplayHasExactValue('[7] statictext = "46"', '46')).toBe(true);
});

test('Display normalization never causes raw AX body or pixels to enter the report', () => {
  const clean = sanitizeDesktopReport({ observed_tool_error_count: 2, observed_command_failure_count: 3,
    observations: { call: { generation: 141, result: { text: 'PRIVATE_AX_BODY' } } },
    raw_tool_results: { call: 'PRIVATE_PIXELS' }, failure_cleanup_request: { request_id: 'exact-cancel-id' } });
  expect(clean.observed_tool_error_count).toBe(2);expect(clean.observed_command_failure_count).toBe(3);
  expect(clean.observations.call.generation).toBe(141);expect(clean.failure_cleanup_request.request_id).toBe('exact-cancel-id');
  expect(JSON.stringify(clean)).not.toContain('PRIVATE_');
});

test('Calculation Clear requires one exact native button and rejects history or ambiguous candidates', () => {
  expect(calculatorClearButtonRef('[14] button "清除"\n[15] button "Clear History"')).toBe(14);
  expect(calculatorClearButtonRef('[8] button "Clear"\n[15] button "Reset History"')).toBe(8);
  expect(calculatorClearButtonRef('[19] button "AC"')).toBe(19);
  expect(calculatorClearButtonRef('[11] button "全部清除"\n[15] button "清除历史记录"')).toBe(11);
  expect(() => calculatorClearButtonRef('[7] button "Clear History"')).toThrow();
  expect(() => calculatorClearButtonRef('[7] statictext "Clear"')).toThrow();
  expect(() => calculatorClearButtonRef('[7] button "Clear"\n[8] button "清除"')).toThrow();
});

test('Background focus accepts only exact target activation receipts and rejects already-focused', () => {
  expect(calculatorActivationReceipt('Activated window 42 via macOS AppKit. activated the exact current-process window of pid 73 through main-queue AppKit', 42, 73)).toBe('main_queue_appkit_current_process_exact_target');
  expect(calculatorActivationReceipt('Activated window 42 via macOS Accessibility. activated the selected window of pid 73 through AppKit', 42, 73)).toBe('appkit_exact_target');
  expect(calculatorActivationReceipt('Activated window 42 via macOS Accessibility. activated the exact selected window of pid 73', 42, 73)).toBe('appkit_and_ax_exact_target');
  for (const text of ['Activated window 42 via macOS Accessibility. the selected window of pid 73 already has keyboard focus',
    'Activated window 43 via macOS Accessibility. activated the selected window of pid 73 through AppKit',
    'Activated window 42 via macOS Accessibility. activated the selected window of pid 74 through AppKit',
    'Activated window 42 via macOS AppKit. the exact current-process window of pid 73 already has keyboard focus',
    'activated the selected window of pid 73 through AppKit',
    'Activated window 42 via macOS Accessibility. activated the selected window of pid 73 through AppKit extra']) {
    expect(calculatorActivationReceipt(text, 42, 73)).toBeUndefined();
  }
});
