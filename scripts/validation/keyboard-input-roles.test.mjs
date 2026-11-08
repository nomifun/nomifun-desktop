import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyActorInputRoles, verifyWindowInputRoles } from './keyboard-input-roles.mjs';

function actorEvidence() {
  return {
    software_shortcuts: { toggle_preserves_document: true },
    external_keyboard_adapter: {
      k1_toggles_main_window: true,
      k1_preserves_subtitle_and_document: true,
      k1_preserves_active_capture: true,
      k4_reuses_shared_dictation: true,
    },
  };
}

test('native Actor evidence keeps physical main-window and software subtitle roles distinct', () => {
  verifyActorInputRoles(actorEvidence());
  assert.throws(() => verifyActorInputRoles({
    software_shortcuts: { toggle_preserves_document: true },
    external_keyboard_adapter: { k1_reuses_preserving_subtitle_toggle: true },
  }), /k1_toggles_main_window/);
});

test('main-window visibility alone cannot stand in for subtitle or live-capture isolation', () => {
  for (const key of ['k1_preserves_subtitle_and_document', 'k1_preserves_active_capture']) {
    const report = actorEvidence();
    report.external_keyboard_adapter[key] = false;
    assert.throws(() => verifyActorInputRoles(report), new RegExp(key));
  }
});

test('hardware K4 dictation cannot stand in for the independent software subtitle toggle', () => {
  const report = actorEvidence();
  report.software_shortcuts.toggle_preserves_document = false;
  assert.throws(() => verifyActorInputRoles(report), /software subtitle-toggle/);
  const missingK4 = actorEvidence();
  delete missingK4.external_keyboard_adapter.k4_reuses_shared_dictation;
  assert.throws(() => verifyActorInputRoles(missingK4), /k4_reuses_shared_dictation/);
});

test('Linux window witness requires native main activation, hide and untouched subtitle', () => {
  const report = { k1ActivatedMain: true, k1HidMain: true, k1PreservedSubtitle: true };
  verifyWindowInputRoles(report);
  assert.throws(() => verifyWindowInputRoles({ ok: true, automaticShowMapped: true }), /k1ActivatedMain/);
  assert.throws(() => verifyWindowInputRoles({ ...report, k1PreservedSubtitle: false }), /k1PreservedSubtitle/);
  assert.throws(() => verifyWindowInputRoles({ ...report, k1HidMain: 'true' }), /k1HidMain/);
});
