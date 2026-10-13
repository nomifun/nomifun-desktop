#!/usr/bin/env node
// Read-only witnesses for the separately launched REAL Desktop renderer/WK child.
// No native snapshot API, host trust secret, emitted shortcut, or mock listener.
// prepare prints disposable page URLs; verify combines their navigation/reload
// witnesses with a separately captured CUA/human GUI receipt. See --help.
import { readFile, writeFile, stat } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';

const args = process.argv.slice(2);
const option = (name, fallback) => { const index = args.indexOf(name); return index < 0 ? fallback : args[index + 1]; };
if (args.includes('--help')) {
  console.log(`Prepare a new fixture and launch the current real Desktop App:
  cargo run -p nomifun-app --example browser_gui_fixture --features browser-use -- /NEW/ROOT --wk-actions
  NOMIFUN_DATA_DIR=/NEW/ROOT bun run dev
Log in normally using the disposable wk-acceptance-auth.json; select fixture.session_id.
  node scripts/validation/run-macos-browser-shortcut-acceptance.mjs --phase prepare --data-dir /NEW/ROOT --output /NEW/ROOT/shortcuts-plan.json
Open the Browser and navigate to the printed initial_url. For EACH key below,
click the WK webpage's 验收姓名 input first; never press it from the toolbar.
  Cmd+L -> enter address_url -> Enter; actual page must navigate.
  Cmd+R -> same native page reloads, with a new profile witness nonce.
  Cmd+T -> new-tab draft and focused address -> enter new_tab_url -> Enter;
            GUI must show two browser tabs.
  Cmd+W -> closes that second tab; original page and real App window remain.
Capture before/after screenshots or AX evidence for native focus and each result.
Write a receipt in the temporary root containing:
  {"source":"cua_native_app","app_pid":PID,"session_id":"ID",
   "keys":[{"key":"Cmd+L","native_page_focused":true,"result":"address_navigation","evidence":["/ABS/PATH.png"]},
           {"key":"Cmd+R","native_page_focused":true,"result":"page_reloaded","evidence":["/ABS/PATH.png"]},
           {"key":"Cmd+T","native_page_focused":true,"result":"second_tab_opened","evidence":["/ABS/PATH.png"]},
           {"key":"Cmd+W","native_page_focused":true,"result":"second_tab_closed_original_visible","evidence":["/ABS/PATH.png"]}],
   "original_tab_visible":true,"browser_tab_count":1,"main_window_visible":true}
  node scripts/validation/run-macos-browser-shortcut-acceptance.mjs --phase verify --data-dir /NEW/ROOT --plan /NEW/ROOT/shortcuts-plan.json --gui-evidence /NEW/ROOT/shortcuts-gui.json --output /NEW/ROOT/shortcuts-report.json
The driver validates receipt fields/files, current App PID and real HTTP witnesses.
It does not replace the GUI review or claim physical-key evidence on its own.
Native input locks, IME, repeated-key and stale-target cases remain separate tests.`);
  process.exit(0);
}
const phase = option('--phase', 'prepare');
if (process.platform !== 'darwin' || !args.includes('--data-dir') || !['prepare', 'verify'].includes(phase)) throw new Error('Use macOS --data-dir /NEW/ROOT --phase prepare|verify');
const dataDir = resolve(option('--data-dir'));
const output = resolve(option('--output', join(dataDir, `wk-shortcuts-${phase}-${Date.now()}-${randomUUID()}.json`)));
const fixture = JSON.parse(await readFile(join(dataDir, 'wk-acceptance.json'), 'utf8'));
if (fixture.mode !== 'wk-actions' || resolve(fixture.data_dir) !== dataDir) throw new Error('Exact disposable --wk-actions root required');
const bound = JSON.parse(await readFile(join(dataDir, 'port.json'), 'utf8'));
if (!Number.isInteger(bound.pid) || bound.pid <= 0) throw new Error('Missing actual App process identity');
process.kill(bound.pid, 0);
const announced = (await stat(join(dataDir, 'port.json'))).mtimeMs;
const response = await fetch(`${fixture.control}/status`, { signal: AbortSignal.timeout(10000) });
if (!response.ok) throw new Error('Disposable fixture control unavailable');
const status = await response.json();
const witnesses = status.witnesses ?? [];
const now = Date.now();
const report = { phase, data_dir: dataDir, app_pid: bound.pid, session_id: fixture.session_id, started_at: new Date(now).toISOString(), scope: 'real_desktop_native_focus_shortcuts', gui_review: 'separate_cua_or_human_evidence', credentials_exported: false };
try {
  if (phase === 'prepare') {
    const prefix = `native-shortcuts-${randomUUID()}`;
    const url = label => { const url = new URL(fixture.page); url.searchParams.set('case', `${prefix}-${label}`); return url.toString(); };
    Object.assign(report, { case_prefix: prefix, baseline_witness_count: witnesses.length, baseline_at_ms: now, app_announcement_ms: announced,
      initial_url: url('initial'), address_url: url('address'), new_tab_url: url('new-tab'),
      prepared: true, passed: false, remaining: 'Perform the real native-focus GUI sequence and verify; preparation is not acceptance.' });
  } else {
    const planPath = option('--plan');
    if (!planPath) throw new Error('Verification requires --plan from this exact running App');
    const plan = JSON.parse(await readFile(resolve(planPath), 'utf8'));
    if (plan.phase !== 'prepare' || plan.data_dir !== dataDir || plan.app_pid !== bound.pid || plan.session_id !== fixture.session_id) throw new Error('Plan is not from this exact App PID, fixture root and Session');
    const matching = label => witnesses.filter(witness => witness.kind === 'profile' && witness.case === `${plan.case_prefix}-${label}` && witness.received_at_ms >= plan.baseline_at_ms);
    const initial = matching('initial'), address = matching('address'), newTab = matching('new-tab');
    if (initial.length !== 1 || newTab.length !== 1 || address.length !== 2 || new Set(address.map(witness => witness.nonce)).size !== 2) throw new Error(`Expected one initial, one new-tab, and two distinct address/reload witnesses; received ${initial.length}/${newTab.length}/${address.length}`);
    if (!(initial[0].received_at_ms <= address[0].received_at_ms && address[0].received_at_ms <= address[1].received_at_ms && address[1].received_at_ms <= newTab[0].received_at_ms)) throw new Error('Witness order differs from the real shortcut sequence');
    report.http_witnesses = { initial, address_navigation: address[0], native_reload: address[1], second_tab_navigation: newTab[0] };
    report.http_witnesses_passed = true;
    const receiptPath = option('--gui-evidence');
    if (!receiptPath) throw new Error('HTTP navigation alone cannot prove native focus, shortcut dispatch or Cmd+W; --gui-evidence is required');
    const receipt = JSON.parse(await readFile(resolve(receiptPath), 'utf8'));
    if (!['cua_native_app', 'human_gui_review'].includes(receipt.source) || receipt.app_pid !== bound.pid || receipt.session_id !== fixture.session_id || receipt.original_tab_visible !== true || receipt.browser_tab_count !== 1 || receipt.main_window_visible !== true) throw new Error('GUI receipt lacks exact App/Session identity and final original-tab/window proof');
    const expected = { 'Cmd+L': 'address_navigation', 'Cmd+R': 'page_reloaded', 'Cmd+T': 'second_tab_opened', 'Cmd+W': 'second_tab_closed_original_visible' };
    for (const [key, result] of Object.entries(expected)) {
      const matches = receipt.keys?.filter(item => item.key === key) ?? [];
      if (matches.length !== 1 || matches[0].native_page_focused !== true || matches[0].result !== result || !Array.isArray(matches[0].evidence) || matches[0].evidence.length === 0) throw new Error(`Missing actual native-focus GUI evidence for ${key}`);
      for (const file of matches[0].evidence) {
        if (typeof file !== 'string' || !file.startsWith('/') || !(await stat(file)).isFile()) throw new Error(`Missing captured GUI artifact for ${key}`);
      }
    }
    report.gui_evidence = { path: resolve(receiptPath), ...receipt };
    report.passed = true;
  }
} catch (error) { report.passed = false; report.error = error.message; process.exitCode = 1; }
report.finished_at = new Date().toISOString();
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
console.log(JSON.stringify({ report: output, phase, passed: report.passed, prepared: report.prepared, app_pid: report.app_pid, session_id: report.session_id,
  ...(phase === 'prepare' ? { initial_url: report.initial_url, address_url: report.address_url, new_tab_url: report.new_tab_url } : {}), error: report.error }));
