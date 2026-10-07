/** Validate bounded evidence from the isolated live IDMM fixture. */
export const IDMM_DEMO_CASES = [
  'off', 'manual_rule', 'background_rule', 'sidecar_question',
  'sensitive_halt', 'provider_pause', 'stalled_recovery',
];
export const IDMM_DEMO_PREFIX = 'NOMIFUN_IDMM_DEMO_CASE ';

export function parseIdmmDemoCase(line) {
  if (!line.startsWith(IDMM_DEMO_PREFIX) || line.length > 24_000) return null;
  try {
    const value = JSON.parse(line.slice(IDMM_DEMO_PREFIX.length));
    if (!IDMM_DEMO_CASES.includes(value.case) || !['pass', 'fail'].includes(value.status) ||
        !/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value.session_id)) return null;
    for (const name of ['elapsed_ms', 'idmm_turns', 'completed_turns', 'sidecar_calls', 'upstream_calls', 'injected_errors', 'injected_stalls']) {
      if (!Number.isSafeInteger(value[name]) || value[name] < 0 || value[name] > 1_000_000) return null;
    }
    const expected = ['off', 'sensitive_halt', 'provider_pause'].includes(value.case) ? 0 : 1;
    const sidecarValid = value.case === 'sidecar_question' ? value.sidecar_calls >= 1 && value.sidecar_calls <= 3 : value.sidecar_calls === 0;
    if ((value.status === 'pass' && (value.idmm_turns !== expected || !sidecarValid)) ||
        !Array.isArray(value.turns) || value.turns.length > 3 ||
        !Array.isArray(value.interventions) || value.interventions.length > 10 ||
        !Array.isArray(value.transcript) || value.transcript.length > 4) return null;
    for (const item of value.transcript) {
      if (!['accepted', 'completed'].includes(item.state) || typeof item.content !== 'string' || item.content.length > 3000) return null;
    }
    // The Rust producer audits all content against the supplied secret before
    // emission. The console forwards only fixed categories and counts; prose is
    // retained solely in the optional local report.
    if (value.failure_code != null && (typeof value.failure_code !== 'string' || !/^[A-Z0-9_]{1,96}$/.test(value.failure_code))) return null;
    if (value.status === 'pass' && value.case === 'provider_pause' &&
        (value.native_resumes !== 1 || value.known_limitation !== 'provider_pause_requires_explicit_native_resume')) return null;
    return Object.fromEntries(['case', 'status', 'failure_code', 'session_id', 'elapsed_ms', 'idmm_turns',
      'completed_turns', 'sidecar_calls', 'upstream_calls', 'injected_errors', 'injected_stalls',
      'native_resumes', 'known_limitation', 'turns', 'interventions', 'transcript'].map(key => [key, value[key]]));
  } catch { return null; }
}

export function parseIdmmDemoEvidence(output) {
  const cases = output.split(/\r?\n/).map(parseIdmmDemoCase).filter(Boolean);
  if (cases.length !== IDMM_DEMO_CASES.length || cases.some((item, index) => item.case !== IDMM_DEMO_CASES[index] || item.status !== 'pass')) {
    throw new Error('IDMM_DEMO_EVIDENCE_INCOMPLETE');
  }
  if (new Set(cases.map(item => item.session_id)).size !== cases.length ||
      cases.find(item => item.case === 'provider_pause').injected_errors < 1 ||
      cases.find(item => item.case === 'stalled_recovery').injected_stalls !== 1) {
    throw new Error('IDMM_DEMO_EVIDENCE_INVALID');
  }
  return {schema_version: 1, provider: 'stepfun-plan', model: 'step-3.7-flash',
    upstream: 'https://api.stepfun.com/step_plan/v1', status: 'pass_with_limitations',
    limitations:['provider_pause_requires_explicit_native_resume'], cases};
}
