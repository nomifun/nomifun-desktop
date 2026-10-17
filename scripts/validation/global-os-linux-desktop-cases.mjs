import assert from 'node:assert/strict';
import { randomUUID, createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { lstat, readFile, readdir, readlink, realpath } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { readNativeOwnedFixtureBytes } from './global-os-native.mjs';
import { sanitizeDesktopReport } from './global-os-desktop-cases.mjs';

const execute = promisify(execFile);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const gone = pid => { try { process.kill(pid, 0); return false; } catch (error) { if (error.code === 'ESRCH') return true; throw error; } };
const textOf = value => { assert.equal(typeof value.result?.text, 'string'); return value.result.text; };
const observeGeneration = value => { assert(Number.isSafeInteger(value.generation) && value.generation > 0); return value.generation; };
const parseEnvironment = bytes => Object.fromEntries(bytes.toString('utf8').split('\0').filter(Boolean).map(entry => { const index = entry.indexOf('='); return [entry.slice(0, index), entry.slice(index + 1)]; }));

export async function verifyPrivateX11Session({ root, appPid, nativeDataRoot }) {
  assert.equal(process.platform, 'linux'); assert(process.getuid() > 0);
  assert(/^\/tmp\/nomifun-global-os-x11-[0-9a-f-]{36}$/.test(root), 'Desktop effects require the dedicated private X11 session root');
  assert.equal(await realpath(root), root);
  const metadata = await lstat(root);
  assert(metadata.isDirectory() && !metadata.isSymbolicLink() && metadata.uid === process.getuid() && (metadata.mode & 0o777) === 0o700);
  assert.equal((await readNativeOwnedFixtureBytes(join(root, '.nomifun-global-os-x11-session'), 4096)).toString('utf8'), 'nomifun-global-os-private-x11-session.v1\n');
  const session = JSON.parse((await readNativeOwnedFixtureBytes(join(root, 'environment.json'), 16384)).toString('utf8'));
  assert.equal(session.schema, 'nomifun.global-os-x11-session.v1'); assert.equal(session.app_pid, appPid);
  assert.equal(session.native_data_root, nativeDataRoot); assert.equal(session.tcp_disabled, true);
  assert.equal(session.xauthority, join(root, 'Xauthority')); assert(/^:\d+$/.test(session.display));
  assert.equal(session.screen?.width, 1280); assert.equal(session.screen?.height, 800);
  assert.equal(process.env.DISPLAY, session.display); assert.equal(process.env.XAUTHORITY, session.xauthority);
  await readNativeOwnedFixtureBytes(session.xauthority, 16384);
  const appEnvironment = parseEnvironment(await readFile(`/proc/${appPid}/environ`));
  assert.equal(appEnvironment.DISPLAY, session.display); assert.equal(appEnvironment.XAUTHORITY, session.xauthority);
  assert(typeof session.session_bus_address === 'string' && session.session_bus_address.startsWith(`unix:path=${root}/`), 'A real private session bus is required for desktop/A11Y isolation');
  assert(typeof session.at_spi_bus_address === 'string' && session.at_spi_bus_address.startsWith('unix:'), 'The private session bus must publish its actual A11Y bus');
  assert.equal(appEnvironment.DBUS_SESSION_BUS_ADDRESS, session.session_bus_address);
  assert.equal(appEnvironment.AT_SPI_BUS_ADDRESS, session.at_spi_bus_address);
  assert(Number.isSafeInteger(session.session_bus_pid) && session.session_bus_pid > 1);
  assert.equal((await lstat(`/proc/${session.session_bus_pid}`)).uid, process.getuid());
  assert.equal(basename(await readlink(`/proc/${session.session_bus_pid}/exe`)), 'dbus-daemon');
  const busAddress = await execute('/usr/bin/gdbus', ['call', '--address', session.session_bus_address, '--dest', 'org.a11y.Bus', '--object-path', '/org/a11y/bus', '--method', 'org.a11y.Bus.GetAddress'], { encoding: 'utf8', timeout: 10000 });
  const publishedAddress = /^\('([^'\\\r\n]+)',\)\s*$/.exec(busAddress.stdout);
  assert(publishedAddress && publishedAddress[1] === session.at_spi_bus_address, 'A11Y must bind to the actual address returned by this private session bus');
  for (const [key, name] of [['XDG_DATA_HOME', 'xdg-data'], ['XDG_CONFIG_HOME', 'xdg-config'], ['XDG_CACHE_HOME', 'xdg-cache']]) {
    assert.equal(appEnvironment[key], join(root, name), 'The native App toolkit/WebKit state must use this private session');
    const directory = await lstat(join(root, name)); assert(directory.isDirectory() && !directory.isSymbolicLink() && directory.uid === process.getuid() && (directory.mode & 0o777) === 0o700);
  }
  for (const [pid, executable] of [[session.xvfb_pid, 'Xvfb'], [session.window_manager_pid, 'openbox']]) {
    assert(Number.isSafeInteger(pid) && pid > 1 && !gone(pid));
    assert.equal((await lstat(`/proc/${pid}`)).uid, process.getuid());
    assert.equal(basename(await readlink(`/proc/${pid}/exe`)), executable);
  }
  const serverArguments = (await readFile(`/proc/${session.xvfb_pid}/cmdline`)).toString('utf8').split('\0').filter(Boolean);
  const noListen = serverArguments.indexOf('-nolisten'), authority = serverArguments.indexOf('-auth');
  assert(noListen >= 0 && serverArguments[noListen + 1] === 'tcp' && authority >= 0 && serverArguments[authority + 1] === session.xauthority && !serverArguments.includes('-ac'), 'The live private X server must enforce Xauthority and disable TCP');
  const descriptorRoot = `/proc/${session.xvfb_pid}/fd`;
  const descriptors = await Promise.all((await readdir(descriptorRoot)).map(async name => { try { return await readlink(join(descriptorRoot, name)); } catch (error) { if (error.code === 'ENOENT') return undefined; throw error; } }));
  const socketName = `@/tmp/.X11-unix/X${session.display.slice(1)}`;
  assert((await readFile(`/proc/${session.xvfb_pid}/net/unix`, 'utf8')).split('\n').some(line => { const columns = line.trim().split(/\s+/); return columns[7] === socketName && descriptors.includes(`socket:[${columns[6]}]`); }), 'The dedicated Xvfb PID must own the exact abstract Unix display socket');
  const extensions = await execute('/usr/bin/xdpyinfo', ['-queryExtensions'], { encoding: 'utf8', timeout: 5000 });
  assert(!/\bXWAYLAND\b/.test(extensions.stdout), 'The actual server must be genuine native X11, never a Wayland compatibility display');
  return { schema: session.schema, display: session.display, xauthority: session.xauthority, xvfb_pid: session.xvfb_pid, window_manager_pid: session.window_manager_pid, app_pid: appPid, screen: session.screen, session_bus_pid: session.session_bus_pid, private_session_and_a11y_bus_bound: true, tcp_disabled: true, abstract_unix_socket_owned: true, xwayland_extension: false, cookie_retained_in_report: false };
}

export async function createLinuxDesktopScenario({ fixture, workspace, uid, nativeIdentity, nativeDataRoot, x11SessionRoot }) {
  assert.equal(uid, process.getuid()); assert(/^\/tmp\/nomifun-global-os-product-[0-9a-f-]{36}$/.test(fixture)); assert.equal(workspace, join(fixture, 'workspace'));
  const session = await verifyPrivateX11Session({ root: x11SessionRoot, appPid: nativeIdentity.pid, nativeDataRoot });
  const root = join(fixture, 'desktop'), statePath = join(root, 'state.json'), token = randomUUID();
  const entryName = `NomiFun text ${token}`, buttonName = `NomiFun action ${token}`, sinkName = `NomiFun paste sink ${token}`, clearClipboardName = `NomiFun clear clipboard ${token}`;
  const title = `NomiFun Linux Computer fixture ${token}`, typed = `NOMIFUN_TYPED_${token.replaceAll('-', '')}`, axTyped = `NOMIFUN_AX_${token.replaceAll('-', '')}_中文✓`;
  const source = fileURLToPath(new URL('./global-os-linux-gtk-fixture.py', import.meta.url));
  const nativeEvidence = [{ kind: 'private_native_x11_session', ...session }];
  const resources = { root, state_path: statePath, pid: undefined, process_id: undefined, window_id: undefined, started_by_this_case: false, quit_confirmed: false, x11_session_root: x11SessionRoot, scope: 'isolated_real_X11_virtual_desktop_not_WSLg_or_Wayland_grant' };
  let phaseRole = 'desktop-native', generation, entryRef, buttonRef, clearClipboardRef, cursor = 0, state, pixelScale = 1;
  const readState = async () => {
    const value = JSON.parse((await readNativeOwnedFixtureBytes(statePath, 16384)).toString('utf8'));
    assert.equal(value.schema, 'nomifun-linux-computer-fixture-v1'); assert.equal(value.token, token); assert.equal(value.pid, resources.pid); assert.equal(value.title, title);
    assert(Number.isSafeInteger(value.xid) && value.xid > 0);
    for (const key of ['button', 'entry', 'sink', 'scroll']) {
      const bounds = value.widgets[key]; assert(bounds && ['x', 'y', 'width', 'height'].every(name => Number.isFinite(bounds[name])));
      assert(bounds.width > 0 && bounds.height > 0 && bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= 1280 && bounds.y + bounds.height <= 800);
    }
    state = value; return value;
  };
  const awaitState = async predicate => {
    for (let attempt = 0; attempt < 60; attempt++) {
      try { const value = await readState(); if (predicate(value)) return value; }
      catch (error) { if (error.code !== 'ENOENT' && !String(error.message).includes('Fixture file changed while')) throw error; }
      await delay(50);
    }
    throw new Error('The owned GTK fixture did not publish the expected native event state');
  };
  const screenshot = (id, expected) => ({ id, name: 'computer_observe', args: { action: 'screenshot' }, verify: async (value, context) => {
    generation = observeGeneration(value); assert.equal(value.kind, 'computer_screenshot'); assert.equal(value.image_count, 1);
    const dimensions = /Screenshot captured: (\d+)x(\d+) \(display (\d+), scaled from (\d+)x(\d+) physical\)/.exec(value.text);
    assert(dimensions && Number(dimensions[4]) === 1280 && Number(dimensions[5]) === 800, 'Screenshot must belong to the private 1280x800 X11 display');
    const images = [];
    for (const message of context.messages.filter(message => message.role === 'user' && Array.isArray(message.content))) {
      let call;
      for (const part of message.content) {
        if (part.type === 'text') call = /^Attachments returned by tool call (.+): $/.exec(part.text)?.[1];
        else if (call === context.callId && part.type === 'image_url') images.push(part.image_url?.url);
      }
    }
    assert.equal(images.length, 1); assert(images[0].startsWith('data:image/png;base64,'));
    const encoded = images[0].slice('data:image/png;base64,'.length), bytes = Buffer.from(encoded, 'base64');
    assert.equal(encoded.length, value.encoded_bytes); assert(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])));
    assert.equal(bytes.readUInt32BE(16), Number(dimensions[1])); assert.equal(bytes.readUInt32BE(20), Number(dimensions[2]));
    pixelScale = Number(dimensions[1]) / 1280;
    const current = await awaitState(current => !current.closed && (!expected || expected(current)));
    nativeEvidence.push({ step: id, kind: 'actual_native_png_and_fixture_state', generation, width: Number(dimensions[1]), height: Number(dimensions[2]), physical_width: 1280, physical_height: 800, png_sha256: createHash('sha256').update(bytes).digest('hex'), encoded_bytes: encoded.length, pixels_retained: false, pid: current.pid, window_id: current.xid, click_count: current.click_count, entry_text_matches: current.entry_text === typed, enter_count: current.enter_count, scroll_value: current.scroll_value });
  } });
  const input = (id, args, verify) => ({ id, name: 'computer_input', args: () => ({ ...args(), expected_generation: generation }), verify: async value => {
    assert.equal(value.generation, 0); assert(textOf(value).length > 0); await delay(200); await verify?.();
    nativeEvidence.push({ step: id, kind: 'actual_computer_input_receipt', retired_generation: true });
  } });
  const inspect = (id, expected) => ({ id, name: 'computer_inspect', args: { action: 'observe' }, verify: async value => {
    generation = observeGeneration(value);
    const text = textOf(value);
    assert(text.startsWith('Accessibility snapshot (gen '), 'AT-SPI must return a real accessibility snapshot');
    const named = [...text.matchAll(/^\[(\d+)\]\s+(\S+)\s+("(?:\\.|[^"\\])*")(?:\s|$)/gm)];
    const entry = named.filter(match => JSON.parse(match[3]) === entryName);
    const button = named.filter(match => JSON.parse(match[3]) === buttonName);
    assert.equal(entry.length, 1, 'The fresh tree must expose this UUID fixture editable text ref');
    assert.equal(button.length, 1, 'The fresh tree must expose this UUID fixture button ref');
    assert.equal(entry[0][2], 'text'); assert.equal(button[0][2], 'button');
    entryRef = Number(entry[0][1]); buttonRef = Number(button[0][1]);
    const active = await execute('/usr/bin/xprop', ['-root', '_NET_ACTIVE_WINDOW'], { encoding: 'utf8', timeout: 5000 });
    const activeId = /window id # (0x[0-9a-f]+)/i.exec(active.stdout);
    assert(activeId && Number(activeId[1]) === resources.window_id, 'The actual X11 active window must still be this managed fixture');
    assert(entryRef !== buttonRef);
    const sink = named.filter(match => JSON.parse(match[3]) === sinkName);
    const clearClipboard = named.filter(match => JSON.parse(match[3]) === clearClipboardName);
    assert.equal(sink.length, 1); assert.equal(sink[0][2], 'text');
    assert.equal(clearClipboard.length, 1); assert.equal(clearClipboard[0][2], 'button');
    clearClipboardRef = Number(clearClipboard[0][1]);
    const current = await awaitState(current => !current.closed && (!expected || expected(current)));
    nativeEvidence.push({ step: id, kind: 'actual_at_spi_tree', generation, exact_fixture_window: true, entry_ref: entryRef, button_ref: buttonRef, pid: current.pid, window_id: current.xid, entry_text_matches_ax_value: current.entry_text === axTyped, click_count: current.click_count, private_tree_text_retained: false });
  } });
  const semanticInput = (id, args, expected, receipt) => ({ id, name: 'computer_input', args: () => ({ ...args(), expected_generation: generation }), verify: async value => {
    assert.equal(value.generation, 0);
    const text = textOf(value);
    assert(receipt().test(text), 'The exact AT-SPI native semantic receipt is required; pixel fallback does not prove semantic execution');
    assert(!/fell back|pixel click|did not succeed/i.test(text));
    const current = await awaitState(expected);
    nativeEvidence.push({ step: id, kind: 'actual_at_spi_semantic_effect', action: args().action, ref: args().ref, native_semantic_receipt: true, retired_generation: true, entry_text_matches_ax_value: current.entry_text === axTyped, click_count: current.click_count });
  } });
  const centre = widget => {
    const bounds = state.widgets[widget];
    return { x: Math.round((bounds.x + bounds.width / 2) * pixelScale), y: Math.round((bounds.y + bounds.height / 2) * pixelScale) };
  };
  const steps = [
    { id: 'desktop-linux-launch', name: 'start_process', args: { command: '/usr/bin/python3', args: [source, '--root', root, '--token', token], cwd: workspace, env: { DISPLAY: session.display, XAUTHORITY: session.xauthority, GDK_BACKEND: 'x11' }, wait_ms: 0, timeout_ms: 120000 }, verify: async value => {
      assert.equal(value.state, 'running'); assert(Number.isSafeInteger(value.pid) && value.pid > 1); resources.pid = value.pid; resources.process_id = value.process_id; resources.started_by_this_case = true; cursor = value.output.next_cursor;
      assert.equal((await lstat(`/proc/${value.pid}`)).uid, uid); assert(/^python3(?:\.\d+)?$/.test(basename(await readlink(`/proc/${value.pid}/exe`))));
      nativeEvidence.push({ step: 'desktop-linux-launch', kind: 'actual_managed_ordinary_user_fixture', pid: value.pid, uid });
    } },
    { id: 'desktop-linux-ready', name: 'poll_process', repeatable: true, maxAttempts: 20, args: () => ({ process_id: resources.process_id, cursor, wait_ms: 100 }), verify: async value => { assert.equal(value.state, 'running'); cursor = value.output.next_cursor; await awaitState(value => !value.closed && value.clipboard_initial_empty === true); return true; } },
    { id: 'desktop-linux-window', name: 'computer_observe', args: { action: 'list_windows' }, repeatable: true, maxAttempts: 20, verify: async value => {
      generation = observeGeneration(value); const current = await readState();
      const windows = [...textOf(value).matchAll(/^- id=(\d+) app=("(?:\\.|[^"\\])*") title=("(?:\\.|[^"\\])*") at \((-?\d+), (-?\d+)\) size (\d+)x(\d+)( \[focused\])?$/gm)]
        .filter(match => Number(match[1]) === current.xid && JSON.parse(match[3]) === title);
      if (!windows.length) { await delay(100); return false; } assert.equal(windows.length, 1);
      const pid = await execute('/usr/bin/xprop', ['-id', String(current.xid), '_NET_WM_PID'], { encoding: 'utf8', timeout: 5000 });
      assert.equal(pid.stdout.trim(), `_NET_WM_PID(CARDINAL) = ${resources.pid}`);
      resources.window_id = current.xid; nativeEvidence.push({ step: 'desktop-linux-window', kind: 'actual_window_and_managed_pid', pid: resources.pid, window_id: current.xid, unique_fixture_title_matches: true });
    } },
    input('desktop-linux-focus', () => ({ action: 'focus_window', window_id: resources.window_id })),
    screenshot('desktop-linux-before', value => value.click_count === 0 && value.entry_text === '' && value.enter_count === 0),
    input('desktop-linux-click', () => ({ action: 'left_click', ...centre('button') }), () => awaitState(value => value.click_count === 1)),
    screenshot('desktop-linux-after-click', value => value.click_count === 1),
    input('desktop-linux-entry', () => ({ action: 'left_click', ...centre('entry') })),
    screenshot('desktop-linux-before-type'),
    input('desktop-linux-type', () => ({ action: 'type', text: typed }), () => awaitState(value => value.entry_text === typed)),
    screenshot('desktop-linux-after-type', value => value.entry_text === typed),
    input('desktop-linux-enter', () => ({ action: 'key', key: 'enter' }), () => awaitState(value => value.enter_count === 1)),
    screenshot('desktop-linux-after-enter', value => value.enter_count === 1),
    input('desktop-linux-scroll', () => ({ action: 'scroll', direction: 'down', amount: 5, ...centre('scroll') }), () => awaitState(value => value.scroll_value > 0)),
    screenshot('desktop-linux-after-scroll', value => value.scroll_value > 0),
    inspect('desktop-linux-ax-before-value', value => value.entry_text === typed && value.click_count === 1),
    semanticInput('desktop-linux-ax-set-value', () => ({ action: 'set_element_value', ref: entryRef, text: axTyped }), value => value.entry_text === axTyped && value.click_count === 1, () => new RegExp(`^set value of element \\[${entryRef}\\]\\. Run `)),
    inspect('desktop-linux-ax-before-click', value => value.entry_text === axTyped && value.click_count === 1),
    semanticInput('desktop-linux-ax-click', () => ({ action: 'click_element', ref: buttonRef }), value => value.entry_text === axTyped && value.click_count === 2, () => new RegExp(`^performed action [0-9]+ on element \\[${buttonRef}\\]\\. Run `)),
    inspect('desktop-linux-ax-after-click', value => value.entry_text === axTyped && value.click_count === 2),
    screenshot('desktop-linux-before-copy-focus', value => value.entry_text === axTyped && value.click_count === 0),
    input('desktop-linux-copy-source-focus', () => ({ action: 'left_click', ...centre('entry') })),
    inspect('desktop-linux-before-copy-select'),
    input('desktop-linux-copy-select', () => ({ action: 'key', key: 'ctrl+a' })),
    inspect('desktop-linux-before-copy'),
    input('desktop-linux-copy', () => ({ action: 'key', key: 'ctrl+c' }), () => awaitState(value => value.clipboard_matches_ax_value === true)),
    screenshot('desktop-linux-before-paste-focus'),
    input('desktop-linux-paste-sink-focus', () => ({ action: 'left_click', ...centre('sink') })),
    inspect('desktop-linux-before-paste'),
    input('desktop-linux-paste', () => ({ action: 'key', key: 'ctrl+v' }), async () => {
      const current = await awaitState(value => value.sink_text === axTyped && value.clipboard_matches_ax_value === true);
      nativeEvidence.push({ step: 'desktop-linux-paste', kind: 'actual_private_clipboard_cross_widget_copy_paste', native_keyboard_copy_paste: true, unicode_marker_matches: current.sink_text === axTyped, source_and_sink_distinct: true, initial_clipboard_empty: current.clipboard_initial_empty, scope: 'private_X11_clipboard_only_no_independent_clipboard_API' });
    }),
    inspect('desktop-linux-before-clipboard-restore'),
    semanticInput('desktop-linux-clipboard-restore', () => ({ action: 'click_element', ref: clearClipboardRef }), value => value.clipboard_empty === true && value.clipboard_restore_count === 1 && value.sink_text === axTyped, () => new RegExp(`^performed action [0-9]+ on element \\[${clearClipboardRef}\\]\\. Run `)),
    inspect('desktop-linux-after-clipboard-restore', value => value.clipboard_empty === true && value.clipboard_restore_count === 1 && value.sink_text === axTyped),
    input('desktop-linux-quit', () => ({ action: 'key', key: 'ctrl+q' }), () => awaitState(value => value.closed)),
    { id: 'desktop-linux-reaped', name: 'poll_process', repeatable: true, maxAttempts: 30, args: () => ({ process_id: resources.process_id, cursor, wait_ms: 100 }), verify: async value => {
      cursor = value.output.next_cursor; if (value.state === 'running') return false;
      assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup.reaped, true); assert(gone(resources.pid));
      const current = await readState(); assert.equal(current.closed, true); assert.equal(current.entry_text, axTyped); assert.equal(current.click_count, phaseRole === 'desktop-native' ? 2 : 0); assert.equal(current.sink_text, phaseRole === 'desktop-native' ? '' : axTyped); assert.equal(current.clipboard_empty, true); assert.equal(current.clipboard_restore_count, phaseRole === 'desktop-native' ? 0 : 1); resources.quit_confirmed = true;
      nativeEvidence.push({ step: 'desktop-linux-reaped', kind: 'native_fixture_normal_exit', pid: resources.pid, exit_code: 0, reaped: true, closed: true, click_count: current.click_count, entry_text_matches_ax_value: current.entry_text === axTyped, sink_text_matches_unicode: current.sink_text === axTyped, clipboard_restored_empty: current.clipboard_empty, clipboard_restore_count: current.clipboard_restore_count, enter_count: current.enter_count, scroll_value: current.scroll_value });
    } },
    { id: 'desktop-linux-delete', name: 'delete_path', args: { path: root }, verify: async () => { await assert.rejects(lstat(root), { code: 'ENOENT' }); } },
  ];
  // Each normal user Turn owns and reaps its own managed GTK process. Keeping
  // both flows below the default model-step budget requires no added grants,
  // capability/permission mode, retained temporary process or budget override.
  const copyStart = steps.findIndex(candidate => candidate.id === 'desktop-linux-before-copy-focus');
  const quitStart = steps.findIndex(candidate => candidate.id === 'desktop-linux-quit');
  const lifecycleStart = steps.slice(0, 4), lifecycleEnd = steps.slice(quitStart);
  const phaseLaunch = role => ({ ...lifecycleStart[0], args: () => { phaseRole = role; resources.quit_confirmed = false; return lifecycleStart[0].args; } });
  const nativeSteps = [phaseLaunch('desktop-native'), ...steps.slice(1, copyStart), ...lifecycleEnd];
  const clipboardSteps = [phaseLaunch('desktop-clipboard'), ...lifecycleStart.slice(1),
    inspect('desktop-linux-clipboard-initial-tree', value => value.entry_text === '' && value.sink_text === '' && value.clipboard_initial_empty === true),
    semanticInput('desktop-linux-clipboard-source-value', () => ({ action: 'set_element_value', ref: entryRef, text: axTyped }), value => value.entry_text === axTyped && value.click_count === 0, () => new RegExp(`^set value of element \\[${entryRef}\\]\\. Run `)),
    ...steps.slice(copyStart, quitStart), ...lifecycleEnd];
  const phases = [
    { role: 'desktop-native', steps: nativeSteps, prompt: `Only operate the host-allocated private GTK fixture ${root} on this private X11 display. Launch it through the formal managed process tool, identify its exact window/PID, focus it, and use fresh Computer screenshots/generations for click/type/Enter/scroll. Observe its real AT-SPI tree with UUID text/button names, use the fresh text ref for native whole-value replacement with ${axTyped}, then a fresh button ref for native Action click. Pixel fallback does not prove those semantic actions. Close only this GTK with Computer Ctrl+q, poll the exact managed process to exit0/reaped/PID absent, and delete its fixture. Leave the initial empty private clipboard untouched; this Turn does not retain a temporary GTK process.` },
    { role: 'desktop-clipboard', steps: clipboardSteps, prompt: `Start a fresh managed GTK fixture at the same now-absent owned path ${root}, identify/focus its exact new PID/window, and prove its private clipboard starts empty. Through real AT-SPI whole-value setting put ${axTyped} into the source entry. Use fresh Computer observations and pointer/key actions for select-all/Ctrl+C, focus the distinct sink and Ctrl+V; independently require both the private clipboard oracle and sink to match the complete Unicode value. Use a fresh AX ref/native button Action to restore the initially empty private clipboard, verify empty again, then Computer Ctrl+q, exact process exit0/reaped/PID absence and formal file deletion. Do not touch any real user clipboard, WSLg desktop or other app; do not claim a separate clipboard read/write API.` },
  ];
  const safeFailure = error => ({ name: error?.name ?? 'Error', code: /\b(?:COMPUTER|SYSTEM_PERMISSION|ROLE_HOST)_[A-Z_]+\b/.exec(String(error?.message ?? ''))?.[0], step: error?.safe_step, message: 'The isolated Linux default-Agent desktop scenario did not complete. Private pixels, provider bodies and global window text are omitted; inspect canonical call identities and scoped fixture evidence.' });
  return { title: 'Default General Agent on a private native X11 virtual desktop', modelTraits: ['vision_input'], resources, steps, phases, nativeEvidence,
    prompt: `Only operate this host-allocated isolated GTK fixture ${root} on the dedicated private X11 display. Start the supplied Python fixture through the existing managed process tool with token ${token}, prove its exact window/PID, then use only Computer tools to focus and capture real screenshots; click its button once, focus its entry and type ${typed}, press Enter and scroll its own pane. Take a fresh screenshot observation before each subsequent pixel Computer input and verify the fixture's own event state. Coordinates come from this controlled fixture's native widget bounds, so this is deterministic product-path evidence rather than live vision reasoning. Then use computer_inspect accessibility observations to obtain this exact fixture window and its unique text/button refs. Use set_element_value on the fresh text ref to replace its whole value with ${axTyped}, obtain a new tree, use click_element on the fresh button ref for the second click, and obtain another tree. Require native AT-SPI semantic receipts and independently verify the exact full value and click count two; pixel fallback is a failure for these semantic steps. Next use fresh observations and formal Computer pointer/key actions to focus the source, select all, Ctrl+C, focus the distinct paste sink and Ctrl+V. The private clipboard oracle and sink must match the complete Unicode marker. Observe fresh AX refs and click the restore-private-clipboard button; independently prove the original empty private clipboard is restored. This proves native GUI clipboard copy/paste, not an independent clipboard read/write API. Close only this fixture with Ctrl+q, poll the same process to normal exit and cleanup.reaped, verify its exact PID is gone, then delete this run's fixture directory. Do not use shell/UI scripts to inject input, touch the real WSLg desktop, touch the real user clipboard or daily files, or claim Wayland grant coverage.`,
    completionSummary: role => role === 'desktop-native'
      ? 'This normal user Turn used the unmodified default General Agent and formal Computer/native process/file owners on the private X11 display. It proved exact managed window/PID, real PNG vision attachments, native focus/click/type/Enter/scroll, fresh UUID-named AT-SPI refs, native whole-value replacement and native Action click without pixel fallback. Its own GTK events matched the expected full value and two clicks. Computer Ctrl+q plus exact process poll established normal exit0/reaped/PID absent, then its fixture was deleted. The private clipboard stayed initially empty. A later separate normal user Turn owns the clipboard scenario; no temporary process is retained across Turns.'
      : role === 'desktop-clipboard'
        ? 'This separate normal user Turn used the same default General Agent and Session with a fresh managed GTK lifecycle. Real AT-SPI refs/native EditableText put the full Unicode value into the source; formal Computer Ctrl+C/Ctrl+V copied it across distinct source/sink controls, and the fixture clipboard oracle/sink matched it. A fresh native AX button restored the initial empty private clipboard. Computer Ctrl+q and exact process poll proved exit0/reaped/PID absent before formal deletion. This is private native X11 GUI copy/paste evidence, without an independent clipboard API, actual WSLg/Wayland authorization or external-model reasoning claim.'
        : 'The earlier failure remains failed; only canonical cancellation/cleanup of this exact owned temporary fixture is relevant.',
    cleanupNeeded: () => false, cleanupSteps: [], cleanupIntent: { prerequisite: 'Cancel the exact original canonical Turn and prove process-tree cleanup before any further work. No host UI injection or raw PID kill.', temporary_process: () => ({ process_id: resources.process_id, observed_pid: resources.pid, action: 'The original canonical Turn owner cancels/reaps its temporary GTK process tree.' }) },
    sanitizeReport: report => ({ ...sanitizeDesktopReport(report, nativeEvidence), failure: report.failure ? { name: report.failure.name, code: report.failure.code, step: report.failure.step, message: report.failure.message } : undefined }), sanitizeFailure: safeFailure,
    pendingAcceptance: [{ id: 'WSLg-global-desktop-and-Wayland-portal-consent', status: 'not_run', reason: 'This complete default-Agent ingress runs on a private genuine X11 virtual desktop. Existing WSLg/XWayland requires its real portal facilities/grant and is a separate acceptance target.' }],
  };
}
