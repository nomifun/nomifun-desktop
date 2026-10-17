// Native Computer acceptance through the default Agent only. Host subprocesses
// below observe processes; they never launch/focus/click/type/quit applications.
// Raw screenshots, AX trees and global window titles must not reach reports.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const calculator = '/System/Applications/Calculator.app';
const executable = `${calculator}/Contents/MacOS/Calculator`;
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const expectedApp = /^(?:Calculator|计算器)$/u;
export const CALCULATOR_DISPLAY_NORMALIZATION = 'Only Unicode Bidi_Control (U+061C, U+200E/U+200F, U+202A..U+202E, U+2066..U+2069) and their RustDebug Unicode escapes are removed from one quoted display token; numeric value matching remains exact.';

export function normalizeCalculatorBidi(value) {
  return value.replace(/[\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/gu, '')
    .replace(/\\u\{(?:0?61c|200[ef]|202[a-e]|206[6-9])\}/gi, '');
}

export function calculatorDisplayHasExactValue(text, expected) {
  if (typeof expected !== 'string' || !/^(?:0|[1-9]\d*)$/.test(expected)) return false;
  return calculatorDisplayHasExactQuotedToken(text, expected);
}

export function calculatorDisplayHasExactAdditionToken(text, expected) {
  // The native case established full-expression displays (12+, then 12+3).
  // Require that same exact shape for the next digit as well; an expression
  // is never accepted as a substitute for its final value 46.
  if (!['12+', '12+3', '12+34'].includes(expected)) return false;
  return calculatorDisplayHasExactQuotedToken(text, expected);
}

function calculatorDisplayHasExactQuotedToken(text, expected) {
  return text.split('\n').some(line => {
    // Parse the actual format_entries structure before normalizing a quoted
    // token. App identity, roles, refs and native generation stay untouched.
    const entry = /^\[(\d+)\][ \t]+([A-Za-z][A-Za-z0-9_-]*)(?:[ \t]+("(?:\\.|[^"\\\r\n])*"))?(?:[ \t]+=[ \t]+("(?:\\.|[^"\\\r\n])*"))?(?:[ \t]+\[[^\]\r\n]*\])?[ \t]*$/.exec(line);
    if (!entry || entry[2].toLowerCase().includes('button')) return false;
    // When AXValue exists, an unrelated numeric name is never its substitute.
    const token = entry[4] ?? entry[3];
    return token !== undefined && normalizeCalculatorBidi(token) === `"${expected}"`;
  });
}

export function calculatorClearButtonRef(text) {
  // Match calculation Clear only, never a history/reset command. The native
  // fixture established the current Chinese label; fixed English equivalents
  // allow the same public Calculator action in the other supported locale.
  const labels = new Set(['清除', '全部清除', 'Clear', 'All Clear', 'Clear All', 'Clear Entry', 'C', 'AC']);
  const refs = [...text.matchAll(/^\[(\d+)\]\s+button\s+"([^"\\\r\n]+)"(?:\s|$)/gm)]
    .filter(match => labels.has(match[2])).map(match => Number(match[1]));
  assert.equal(refs.length, 1, 'The actual Calculator AX snapshot must expose one unambiguous calculation Clear button');
  return refs[0];
}

export function calculatorActivationReceipt(text, windowId, pid) {
  if (text === `Activated window ${windowId} via macOS AppKit. activated the exact current-process window of pid ${pid} through main-queue AppKit`) return 'main_queue_appkit_current_process_exact_target';
  if (text === `Activated window ${windowId} via macOS Accessibility. activated the selected window of pid ${pid} through AppKit`) return 'appkit_exact_target';
  if (text === `Activated window ${windowId} via macOS Accessibility. activated the exact selected window of pid ${pid}`) return 'appkit_and_ax_exact_target';
  return undefined;
}

export function probeAcceptanceHostWindow(identity) {
  assert(Number.isInteger(identity?.pid) && identity.pid > 1, 'The actual API caller PID must bind host setup');
  assert.equal(identity.bundle_identifier, 'com.nomifun.desktop');
  assert(/^\/private\/tmp\/nomifun-global-os-owner-[A-Za-z0-9-]+\/native-data$/.test(identity.native_data_root), 'Host setup requires the marked private native dataset');
  const result = spawnSync('/usr/bin/swift', [fileURLToPath(new URL('./global-os-desktop-window-probe.swift', import.meta.url)),
    String(identity.pid), identity.bundle_identifier], { encoding: 'utf8', timeout: 30000, maxBuffer: 65536 });
  assert.equal(result.status, 0, 'Read-only dedicated acceptance window metadata probe failed');
  const value = JSON.parse(result.stdout);
  assert.equal(value.schema, 'nomifun.desktop-acceptance-window-probe.v1');
  assert.equal(value.pid, identity.pid);
  assert.equal(value.bundle_identifier, identity.bundle_identifier);
  assert.equal(value.app_path, '/Applications/NomiFun Global OS Acceptance.app');
  assert.equal(value.metadata_only, true); assert.equal(value.ui_effects, false);
  assert(Number.isInteger(value.window?.window_id) && value.window.window_id > 0);
  assert.equal(value.window.layer, 0);
  for (const key of ['x', 'y', 'width', 'height']) assert(Number.isFinite(value.window[key]));
  assert(value.window.width >= 880 && value.window.height >= 600);
  return value;
}

function calculatorPids() {
  const result = spawnSync('/bin/ps', ['-axo', 'pid=,comm='], { encoding: 'utf8', timeout: 5000, maxBuffer: 4 * 1024 * 1024 });
  assert.equal(result.status, 0, 'Read-only Calculator process observation failed');
  return result.stdout.split('\n').flatMap(line => {
    const match = /^\s*(\d+)\s+(.+?)\s*$/.exec(line);
    return match && match[2] === executable ? [Number(match[1])] : [];
  });
}

function gone(pid) {
  try { process.kill(pid, 0); return false; }
  catch (error) { if (error.code === 'ESRCH') return true; throw error; }
}

function resultText(value) {
  assert(value && typeof value.result?.text === 'string', 'Computer returned no actual native text result');
  return value.result.text;
}

function requireObservation(value) {
  assert(Number.isInteger(value.generation) && value.generation > 0, 'Native observation must carry the admitted generation');
  return value.generation;
}

function safeFailure(error) {
  const full = String(error?.message ?? ''), first = full.split('\n')[0];
  const safe = /^(?:Calculator is already running|Calculator appeared before|The fresh Calculator display did not establish expected value (?:\d+|12\+|12\+3|12\+34)$|The actual Calculator AX snapshot must expose one unambiguous digit 1 button$|A fresh AX observation must identify Calculator|Native screenshot is not a PNG$|Screenshot must (?:report actual dimensions|carry one real native image)$|Native Calculator launch did not establish an actual process$)/.test(first);
  const code = /\b(?:SYSTEM_PERMISSION_REQUIRED|ROLE_HOST_[A-Z_]+|COMPUTER_[A-Z_]+)\b/.exec(full)?.[0];
  const safeScalar = value => typeof value === 'number' || typeof value === 'boolean' || value === null ? value : undefined;
  return { name: error?.name ?? 'Error', code,
    step: typeof error?.safe_step === 'string' && /^desktop-[a-z0-9-]+$/.test(error.safe_step) ? error.safe_step : undefined,
    expected: safeScalar(error?.expected), actual: safeScalar(error?.actual),
    ax_error: /AXError (-?\d+)/.test(full) ? Number(/AXError (-?\d+)/.exec(full)[1]) : undefined,
    message: safe ? first : `Native desktop acceptance did not complete${code ? ` (${code})` : ''}. Raw screenshots, AX/window contents and provider error bodies are omitted; inspect scoped safe native evidence and canonical call identities.` };
}

function redact(value, key = '') {
  const error = key === 'failure' || key === 'failure_cleanup' || key.endsWith('_failure') || key.endsWith('_error') || key.endsWith('_unproven');
  if (error && typeof value === 'string') return safeFailure(new Error(value));
  if (value === null || value === undefined || typeof value !== 'object') return value;
  if (key === 'events' && Array.isArray(value)) return value.map(event => ({ seq: event.seq, kind: event.kind,
    correlation_id: event.correlation_id, causation_id: event.causation_id, payload_omitted: true }));
  if (key === 'raw_tool_results' || key === 'observations') return Object.fromEntries(Object.entries(value).map(([id, observation]) => [id,
    { observed: true, generation: observation?.generation, private_native_body_omitted: true }]));
  if (error) return safeFailure(value);
  if (Array.isArray(value)) return value.map(item => redact(item));
  return Object.fromEntries(Object.entries(value).map(([name, item]) => [name,
    ['images', 'image_data', 'image_url'].includes(name) ? { private_pixels_omitted: true } : redact(item, name)]));
}

export const sanitizeDesktopReport = (report, nativeEvidence = []) => ({ ...redact(report), native_desktop_evidence: nativeEvidence,
  privacy: { raw_pixels_retained: false, raw_ax_or_global_window_text_retained: false } });

export function createMacDesktopScenario({ fixture, workspace, uid, desktopBackgroundFocus = false, desktopHostIdentity }) {
  assert.equal(process.platform, 'darwin');
  assert.equal(uid, process.getuid());
  assert.equal(typeof desktopBackgroundFocus, 'boolean');
  assert(/^\/private\/tmp\/nomifun-global-os-product-[0-9a-f-]{36}$/.test(fixture), 'Desktop acceptance requires the driver-owned unique fixture');
  assert.equal(workspace, `${fixture}/workspace`);
  assert.deepEqual(calculatorPids(), [], 'Calculator is already running; this case must not disturb an existing application');
  const hostWindow = desktopBackgroundFocus ? probeAcceptanceHostWindow(desktopHostIdentity) : undefined;
  const nativeEvidence = [];
  const resources = { application: calculator, pid: undefined, window_id: undefined, started_by_this_case: false,
    quit_requested: false, quit_confirmed: false, expected_calculation: desktopBackgroundFocus ? undefined : '12+34=46',
    desktop_mode: desktopBackgroundFocus ? 'background_focus' : 'full', input_receipt_to_fresh_observation_wait_ms: 350 };
  if (hostWindow) resources.host_setup_target = hostWindow;
  let generation, oneRef, clearRef;

  const verifyInput = value => {
    assert.equal(value.generation, 0, 'Input must retire the old observation generation');
    assert(resultText(value).length > 0, 'Native input returned no receipt');
  };
  const settleInput = async value => {
    verifyInput(value);
    // The loopback scripted provider responds much faster than a real model.
    // Let the owned app publish its asynchronous label/value AX notifications
    // before the next fresh observation; the next input still has to pass all
    // live generation/content guards. A stale rejection is never suppressed.
    await delay(resources.input_receipt_to_fresh_observation_wait_ms);
  };
  const inspect = (id, expectedDisplay, chooseOne = false, chooseClear = false) => ({ id, name: 'computer_inspect', args: { action: 'observe' },
    verify: value => {
      generation = requireObservation(value);
      const text = resultText(value);
      assert(/element\(s\) in (?:Calculator|计算器)(?:\.| \(a11y)/u.test(text.split('\n')[0]), 'A fresh AX observation must identify Calculator before any keyboard operation');
      if (chooseOne) {
        const matches = [...text.matchAll(/^\[(\d+)\]\s+button\s+"1"(?:\s|$)/gm)];
        assert.equal(matches.length, 1, 'The actual Calculator AX snapshot must expose one unambiguous digit 1 button');
        oneRef = Number(matches[0][1]);
      }
      if (chooseClear) clearRef = calculatorClearButtonRef(text);
      if (expectedDisplay !== undefined) {
        assert(calculatorDisplayHasExactValue(text, expectedDisplay) || calculatorDisplayHasExactAdditionToken(text, expectedDisplay),
          `The fresh Calculator display did not establish expected value ${expectedDisplay}`);
      }
      nativeEvidence.push({ step: id, kind: 'actual_calculator_ax_observation', admitted_generation: generation,
        digit_one_ref: chooseOne ? oneRef : undefined, verified_display: expectedDisplay,
        calculation_clear_ref: chooseClear ? clearRef : undefined,
        display_semantic_normalization: expectedDisplay === undefined ? undefined : CALCULATOR_DISPLAY_NORMALIZATION });
    } });
  const key = (id, combo) => ({ id, name: 'computer_input', args: () => ({ action: 'key', key: combo, expected_generation: generation }),
    verify: async value => { await settleInput(value); nativeEvidence.push({ step: id, kind: 'actual_native_key_receipt', key: combo }); } });
  const clear = id => ({ id, name: 'computer_input', args: () => ({ action: 'click_element', ref: clearRef, expected_generation: generation }),
    verify: async value => { await settleInput(value); nativeEvidence.push({ step: id, kind: 'actual_native_calculation_clear_click', ref: clearRef }); } });
  const screenshot = id => ({ id, name: 'computer_observe', args: { action: 'screenshot' }, verify: (value, context) => {
    requireObservation(value);
    assert.equal(value.kind, 'computer_screenshot', 'Screenshot must use the real typed image projection');
    assert.equal(value.image_count, 1);
    const text = value.text;
    const dimensions = /Screenshot captured: (\d+)x(\d+) \(display (\d+), scaled from (\d+)x(\d+) physical\)/.exec(text);
    assert(dimensions, 'Screenshot must report actual dimensions');
    const images = [];
    assert(Array.isArray(context?.messages) && typeof context.callId === 'string', 'Screenshot verification needs the actual provider request and exact call identity');
    for (const message of context.messages.filter(message => message.role === 'user' && Array.isArray(message.content))) {
      let call;
      for (const part of message.content) {
        if (part.type === 'text') call = /^Attachments returned by tool call (.+): $/.exec(part.text)?.[1];
        else if (call === context.callId && part.type === 'image_url') images.push(part.image_url?.url);
      }
    }
    assert.equal(images.length, 1, 'Screenshot must carry one real native image for its exact call');
    assert.equal(typeof images[0], 'string');
    assert(images[0].startsWith('data:image/png;base64,'), 'Screenshot attachment must retain its native PNG media type');
    const data = images[0].slice('data:image/png;base64,'.length);
    assert.equal(data.length, value.encoded_bytes, 'Typed screenshot attachment bytes must match its tool receipt');
    const bytes = Buffer.from(data, 'base64');
    assert(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Native screenshot is not a PNG');
    assert.equal(bytes.readUInt32BE(16), Number(dimensions[1]));
    assert.equal(bytes.readUInt32BE(20), Number(dimensions[2]));
    nativeEvidence.push({ step: id, kind: 'actual_native_png', width: Number(dimensions[1]), height: Number(dimensions[2]),
      display: Number(dimensions[3]), physical_width: Number(dimensions[4]), physical_height: Number(dimensions[5]),
      bytes: bytes.length, pixels_retained: false });
  } });
  const window = (id, mode = 'ordinary') => ({ id, name: 'computer_observe', args: { action: 'list_windows' }, repeatable: true, maxAttempts: 8,
    verify: async value => {
      generation = requireObservation(value);
      const current = calculatorPids();
      assert.equal(current.length, 1, 'The owned Calculator process must remain unique');
      assert.equal(current[0], resources.pid, 'Calculator process identity changed');
      const candidates = [...resultText(value).matchAll(/^- id=(\d+) app=("(?:\\.|[^"\\])*") title=("(?:\\.|[^"\\])*") at \((-?\d+), (-?\d+)\) size (\d+)x(\d+)( \[focused\])?$/gm)]
        .map(match => ({ id: Number(match[1]), app: JSON.parse(match[2]), x: Number(match[4]), y: Number(match[5]), width: Number(match[6]), height: Number(match[7]), focused: Boolean(match[8]) }))
        .filter(item => expectedApp.test(item.app) && item.width >= 150 && item.height >= 150);
      if (!candidates.length) return false;
      assert.equal(candidates.length, 1, 'The native window observation must identify one Calculator window');
      if (mode === 'initial' && !candidates[0].focused) { await delay(250); return false; }
      if (mode === 'initial') resources.initial_focused_window_verified = true;
      if (mode === 'background') {
        assert.equal(resources.initial_focused_window_verified, true, 'Background setup requires the same Calculator first observed focused');
        assert.equal(candidates[0].id, resources.window_id, 'The owned Calculator window changed during focus displacement setup');
        if (candidates[0].focused) { await delay(250); return false; }
        resources.background_focus_displacement_observed = true;
        resources.background_witness_generation = generation;
      }
      resources.window_id = candidates[0].id;
      nativeEvidence.push({ step: id, kind: 'owned_calculator_native_window', pid: resources.pid,
        window_id: resources.window_id, x: candidates[0].x, y: candidates[0].y, width: candidates[0].width, height: candidates[0].height,
        focused: candidates[0].focused, admitted_generation: generation, agent_setup_witness: mode === 'background' });
    } });
  const focus = (id, requireActivation = false) => ({ id, name: 'computer_input', args: () => ({ action: 'focus_window', window_id: resources.window_id,
    expected_generation: generation }), verify: async value => {
      await settleInput(value);
      const activation = calculatorActivationReceipt(resultText(value), resources.window_id, resources.pid);
      if (requireActivation) {
        assert.equal(resources.background_focus_displacement_observed, true, 'Background focus requires an actual nonfocused native observation');
        assert(activation, 'Background focus must report actual exact-target native activation; already-focused is insufficient');
        resources.background_activation_verified = true;
        resources.native_activation_path = activation;
      }
      nativeEvidence.push({ step: id, kind: 'actual_native_focus_receipt', window_id: resources.window_id, actual_activation_path: activation });
    } });
  const hostSetup = () => [{ id: 'desktop-background-host-observe', name: 'computer_observe', args: { action: 'list_windows' },
    verify: value => {
      generation = requireObservation(value);
      const matches = [...resultText(value).matchAll(/^- id=(\d+) app=("(?:\\.|[^"\\])*") title=("(?:\\.|[^"\\])*") at \((-?\d+), (-?\d+)\) size (\d+)x(\d+)( \[focused\])?$/gm)]
        .filter(match => Number(match[1]) === hostWindow.window.window_id);
      assert.equal(matches.length, 1, 'The actual native observation must contain the exact dedicated host window');
      for (const [index, key] of [[4, 'x'], [5, 'y'], [6, 'width'], [7, 'height']]) {
        assert(Math.abs(Number(matches[0][index]) - hostWindow.window[key]) <= 2, 'Dedicated host window geometry changed before setup');
      }
      assert.equal(Boolean(matches[0][8]), false, 'Host setup must begin with the exact acceptance window in background');
      nativeEvidence.push({ step: 'desktop-background-host-observe', kind: 'exact_native_acceptance_window_observed',
        pid: hostWindow.pid, window_id: hostWindow.window.window_id, focused: false, admitted_generation: generation });
    } }, { id: 'desktop-background-host-activate', name: 'computer_input',
    args: () => ({ action: 'focus_window', window_id: hostWindow.window.window_id, expected_generation: generation }),
    verify: async value => {
      await settleInput(value);
      const activation = calculatorActivationReceipt(resultText(value), hostWindow.window.window_id, hostWindow.pid);
      assert(activation, 'Dedicated host setup must report actual exact-target activation; already-focused is insufficient');
      resources.host_setup_activation_path = activation;
      nativeEvidence.push({ step: 'desktop-background-host-activate', kind: 'actual_native_acceptance_host_activation',
        pid: hostWindow.pid, window_id: hostWindow.window.window_id, actual_activation_path: activation });
    } }];
  const quit = prefix => [inspect(`${prefix}-fresh-calculator`), { ...key(`${prefix}-cmd-q`, 'cmd+q'), verify: async value => {
    await settleInput(value); resources.quit_requested = true;
  } }, { id: `${prefix}-confirm-exit`, name: 'computer_observe', args: { action: 'wait', seconds: 0.25 }, repeatable: true, maxAttempts: 16,
    verify: () => {
      if (!gone(resources.pid)) return false;
      resources.quit_confirmed = true;
      nativeEvidence.push({ step: `${prefix}-confirm-exit`, kind: 'independent_owned_pid_exit', pid: resources.pid, gone: true });
    } }];
  const steps = [{ id: 'desktop-launch', name: 'launch_app', args: () => {
    assert.deepEqual(calculatorPids(), [], 'Calculator appeared before the Agent launch; do not disturb it');
    return { action: 'launch', target: calculator };
  }, verify: async value => {
    assert(resultText(value).includes('via macOS LaunchServices'), 'App launch must report actual native LaunchServices delivery');
    for (let attempt = 0; attempt < 60; attempt++) {
      const pids = calculatorPids();
      if (pids.length) {
        assert.equal(pids.length, 1, 'Calculator launch produced an ambiguous process identity');
        resources.pid = pids[0]; resources.started_by_this_case = true;
        nativeEvidence.push({ step: 'desktop-launch', kind: 'independent_new_calculator_pid', pid: resources.pid, preexisting: false });
        return;
      }
      await delay(50);
    }
    throw new Error('Native Calculator launch did not establish an actual process');
  } }, window('desktop-window'), focus('desktop-focus'), screenshot('desktop-screenshot-before'),
  // Calculator may restore its prior calculation in a new process. Clear it
  // using its actual calculation Clear AX button. Two bounded presses, each
  // with a fresh ref, handle entry-clear followed by full-clear without
  // relying on global keyboard shortcuts. Require an observed zero before
  // obtaining the fresh digit ref and click; no history/settings are touched.
  inspect('desktop-before-initial-clear', undefined, false, true), clear('desktop-initial-clear'),
  inspect('desktop-before-initial-clear-all', undefined, false, true), clear('desktop-initial-clear-all'), inspect('desktop-find-digit', '0', true),
  { id: 'desktop-click-digit', name: 'computer_input', args: () => ({ action: 'click_element', ref: oneRef, expected_generation: generation }), verify: settleInput },
  inspect('desktop-after-click', '1', false, true), clear('desktop-clear'),
  inspect('desktop-before-clear-all', undefined, false, true), clear('desktop-clear-all'), inspect('desktop-after-clear', '0')];
  for (const [id, combo, display] of [['one', '1', '1'], ['two', '2', '12'], ['plus', 'shift+=', '12+'],
    ['three', '3', '12+3'], ['four', '4', '12+34'], ['equals', 'enter', '46']]) {
    steps.push(key(`desktop-key-${id}`, combo), inspect(`desktop-read-${id}`, display));
  }
  steps.push(screenshot('desktop-screenshot-result'), ...quit('desktop-quit'));
  const scenarioSteps = desktopBackgroundFocus ? [steps[0], window('desktop-background-initial', 'initial'), ...hostSetup(),
    window('desktop-background-displacement', 'background'), focus('desktop-background-activate', true),
    inspect('desktop-background-activated-ax'), ...quit('desktop-background-quit')] : steps;
  return {
    title: desktopBackgroundFocus ? '默认通用 Agent 的后台 Calculator 原生激活回归' : '默认通用 Agent 的原生桌面截图、AX、聚焦、点击与键盘回归', modelTraits: ['vision_input'], steps: scenarioSteps,
    prompt: desktopBackgroundFocus
      ? `仅通过现役Agent工具启动本次新系统Calculator，确认精确窗口最初focused。宿主仅按实际API callerPID和marked数据根绑定并只读识别了专用NomiFun验收App已有唯一主窗口，不按标题猜应用，也未操作UI。Agent以fresh list/generation先真正激活该精确验收host窗口，不启动其他App；再fresh list证实同一Calculator窗口不focused，随后以新generation真正激活Calculator。两个方向都要求实际目标激活回执，already-focused不算通过；新AX必须属于Calculator。最后新AX后CmdQ并确认本次PID退出。不运行算术、不截图，不操作日常App、用户文件、历史、设置或剪贴板。`
      : `仅操作本次新启动的系统 Calculator：先确认没有已运行实例，再通过 launch_app 打开 ${calculator}。使用当前 Computer 工具实际列窗口、聚焦、截图和 AX 观察；确认聚焦Calculator后通过实际AX计算清除按钮两次有界清除可能恢复的旧算式，每次操作前使用新观察和新ref，不清除历史或设置。以新AX观察严格核验显示0并取得新的数字1 ref，再点击并核验显示1。随后同样清除当前计算并通过键盘计算12+34=46，逐步核验当前完整算式及最终独立数值46。最后在新观察后 Cmd+Q 退出本次应用并确认 PID 已退出。不得使用宿主脚本或其他后门执行UI效果，不操作其他应用、不读取/改写用户文件或剪贴板。截图及其他窗口内容仅在运行中使用，验收报告只保留本次Calculator和图片尺寸等必要元数据。`,
    resources, nativeEvidence,
    cleanupNeeded: () => resources.started_by_this_case && !resources.quit_confirmed && !gone(resources.pid),
    get cleanupSteps() { return resources.started_by_this_case && !gone(resources.pid)
      ? [window('desktop-cleanup-window'), focus('desktop-cleanup-focus'), ...quit('desktop-cleanup-quit')] : []; },
    cleanupIntent: { application: calculator, stop_only_owned_application: true, temporary_process: () => resources.pid },
    pendingAcceptance: ['本用例不覆盖首次授予、拒绝、撤回后的许可恢复', '不覆盖麦克风、摄像头或其他设备采集', '不代表 Windows 高完整性桌面或 Linux 桌面验证'],
    completionSummary: () => {
      if (desktopBackgroundFocus) return `默认通用Agent通过现役Computer对本次Calculator PID ${resources.pid}/窗口${resources.window_id}完成独立后台激活用例：初始focused真实观察=${resources.initial_focused_window_verified === true}；以实际API callerPID与marked数据根绑定的专用验收App已有主窗口作为setup，Agent真实激活该精确host窗口路径=${resources.host_setup_activation_path ?? '未验证'}；fresh native list观察Calculator不focused=${resources.background_focus_displacement_observed === true}；Agent以新generation实际目标激活Calculator=${resources.background_activation_verified === true}，路径=${resources.native_activation_path ?? '未验证'}；新AX归属Calculator后CmdQ，独立PID退出=${resources.quit_confirmed}。宿主仅只读窗口元数据，两个方向的UI效果均经Agent，没有CUA或hostUI脚本替代。失败原记录保留，不代表外网模型、首次许可恢复或其他平台通过。`;
      const screenshots = nativeEvidence.filter(item => item.kind === 'actual_native_png').length;
      const clicked = nativeEvidence.some(item => item.step === 'desktop-after-click' && item.verified_display === '1');
      const calculated = nativeEvidence.some(item => item.step === 'desktop-read-equals' && item.verified_display === '46');
      return `默认通用 Agent 经现役 Computer 工具操作本次 Calculator PID ${resources.pid}。已核验的真实PNG截图=${screenshots}张，未保留像素；依据真实AX ref点击数字1并核验显示=${clicked}；新观察代次逐步键入12+34并回车后AX实际显示46=${calculated}；新观察后Cmd+Q和宿主只读PID核验确认退出=${resources.quit_confirmed}。未通过的原验收仍保留失败，恢复关闭不把缺项改成通过。不涉及现有Calculator实例、用户文件或剪贴板；本地脚本供应器不代表外网模型推理验收。`;
    },
    sanitizeReport: report => sanitizeDesktopReport(report, nativeEvidence),
    sanitizeFailure: safeFailure,
  };
}
