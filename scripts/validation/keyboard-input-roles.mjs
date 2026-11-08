// Input-role evidence is shared by all native runners. Physical K1 controls
// the workbench; K4 and the software toggle retain their distinct voice roles.
export function verifyActorInputRoles(report) {
  const external = report.external_keyboard_adapter;
  for (const key of [
    'k1_toggles_main_window',
    'k1_preserves_subtitle_and_document',
    'k1_preserves_active_capture',
    'k4_reuses_shared_dictation',
  ]) {
    if (external?.[key] !== true) throw new Error(`Missing native keyboard input-role evidence: ${key}`);
  }
  if (report.software_shortcuts?.toggle_preserves_document !== true) {
    throw new Error('Missing independent software subtitle-toggle evidence');
  }
}

export function verifyWindowInputRoles(report) {
  for (const key of ['k1ActivatedMain', 'k1HidMain', 'k1PreservedSubtitle']) {
    if (report[key] !== true) throw new Error(`Missing native K1 main-window evidence: ${key}`);
  }
}
