#!/usr/bin/env node
/** Statistical gate only. It does not turn model self-reports or mock trials
 * into evidence, or prove that a supplied task distribution is representative.
 * Input must come from an independently audited product-fixture evaluator.
 */
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const METRICS = ['tools', 'execution', 'quality'];
const MAX_SAMPLES = 100_000;
const MAX_INPUT_BYTES = 64 * 1024 * 1024;
const identifier = value => typeof value === 'string' && /^[a-zA-Z0-9_.:-]{1,128}$/.test(value);
const modelIdentity = value => typeof value === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9._:/@+-]{0,199}$/.test(value);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const requireValue = (valid, message) => { if (!valid) throw new Error(message); };

/** One-sided exact Clopper-Pearson lower bound. Invert the binomial upper
 * tail in log space, including zero failures and very small probabilities.
 * No normal approximation that reports 100% certainty after a few successes.
 */
export function exactLowerBound(successes, total, alpha = 0.05) {
  requireValue(Number.isSafeInteger(total) && total >= 0 && total <= MAX_SAMPLES
    && Number.isSafeInteger(successes) && successes >= 0 && successes <= total
    && Number.isFinite(alpha) && alpha > 0 && alpha < 1, 'Invalid confidence interval input');
  if (successes === 0) return 0;
  if (successes === total) return Math.exp(Math.log(alpha) / total);
  let logChoose = 0;
  for (let i = 1; i <= Math.min(successes, total - successes); i++) {
    logChoose += Math.log(total - i + 1) - Math.log(i);
  }
  const logTail = probability => {
    const logP = Math.log(probability);
    const logQ = Math.log1p(-probability);
    let term = logChoose + successes * logP + (total - successes) * logQ;
    let sum = term;
    for (let k = successes; k < total; k++) {
      term += Math.log(total - k) - Math.log(k + 1) + logP - logQ;
      const maximum = Math.max(sum, term);
      sum = maximum + Math.log1p(Math.exp(Math.min(sum, term) - maximum));
    }
    return sum;
  };
  let lower = 0;
  let upper = successes / total;
  for (let iteration = 0; iteration < 70; iteration++) {
    const middle = (lower + upper) / 2;
    if (logTail(middle) >= Math.log(alpha)) upper = middle;
    else lower = middle;
  }
  return lower;
}

function validate(input) {
  requireValue(object(input) && input.schema_version === 1, 'Expected reliability evidence schema_version=1');
  requireValue(identifier(input.suite_id), 'Invalid suite identity');
  requireValue(typeof input.runtime_build_digest === 'string' && /^[a-f0-9]{64}$/.test(input.runtime_build_digest), 'Expected one exact runtime build digest');
  requireValue(modelIdentity(input.model), 'Expected one explicit frozen model identity');
  requireValue(Array.isArray(input.strata) && input.strata.length > 0 && input.strata.length <= 64, 'Expected a frozen task-stratum manifest');
  requireValue(Array.isArray(input.samples) && input.samples.length <= MAX_SAMPLES, 'Invalid or excessive trial collection');
  requireValue(Array.isArray(input.scheduled_trials) && input.scheduled_trials.length <= MAX_SAMPLES, 'Expected the predeclared trial schedule, including failed and missing runs');
  const strata = new Map();
  for (const stratum of input.strata) {
    requireValue(object(stratum) && identifier(stratum.id) && !strata.has(stratum.id), 'Invalid or repeated task stratum');
    requireValue(Number.isSafeInteger(stratum.minimum_samples) && stratum.minimum_samples > 0
      && stratum.minimum_samples <= MAX_SAMPLES, 'Each stratum needs a positive sample floor');
    requireValue(Number.isSafeInteger(stratum.minimum_duration_ms) && stratum.minimum_duration_ms >= 0, 'Each stratum needs an explicit duration floor');
    requireValue(Array.isArray(stratum.metrics) && stratum.metrics.length > 0
      && new Set(stratum.metrics).size === stratum.metrics.length
      && stratum.metrics.every(metric => METRICS.includes(metric)), 'Invalid applicable metric set');
    requireValue(object(stratum.checks), 'Each stratum needs independent acceptance checks');
    for (const metric of stratum.metrics) {
      const checks = stratum.checks[metric];
      requireValue(Array.isArray(checks) && checks.length > 0 && checks.length <= 128
        && new Set(checks).size === checks.length && checks.every(identifier), 'Invalid acceptance-check manifest');
    }
    requireValue(Object.keys(stratum.checks).every(metric => stratum.metrics.includes(metric)), 'Undeclared metric checks');
    strata.set(stratum.id, stratum);
  }
  requireValue(METRICS.every(metric => input.strata.some(stratum => stratum.metrics.includes(metric))), 'All three reliability metrics must be covered');
  const scheduled = new Map();
  for (const trial of input.scheduled_trials) {
    requireValue(object(trial) && identifier(trial.id) && !scheduled.has(trial.id)
      && strata.has(trial.stratum), 'Invalid or duplicate scheduled trial');
    scheduled.set(trial.id, trial.stratum);
  }
  const ids = new Set();
  const sessions = new Set();
  for (const sample of input.samples) {
    requireValue(object(sample) && identifier(sample.id) && !seenBefore(ids, sample.id), 'Invalid or duplicate trial identity');
    requireValue(identifier(sample.session_id) && !seenBefore(sessions, sample.session_id), 'Turns from one Session cannot count as independent task trials');
    requireValue(sample.runtime_build_digest === input.runtime_build_digest && sample.model === input.model
      && sample.suite_id === input.suite_id, 'Cannot pool different builds, models or evaluation suites');
    requireValue(sample.source === 'live_product' && sample.grader === 'independent_assertions', 'Mock runs and model self-reports are not statistical task evidence');
    const stratum = strata.get(sample.stratum);
    requireValue(Boolean(stratum), 'Trial belongs to an undeclared task stratum');
    requireValue(scheduled.get(sample.id) === sample.stratum, 'Cannot select unscheduled trials or move them between strata');
    requireValue(Number.isSafeInteger(sample.duration_ms) && sample.duration_ms >= 0, 'Invalid trial duration');
    requireValue(object(sample.checks) && Object.keys(sample.checks).length === stratum.metrics.length
      && Object.keys(sample.checks).every(metric => stratum.metrics.includes(metric)), 'Trial metric coverage differs from its manifest');
    for (const metric of stratum.metrics) {
      const outcomes = sample.checks[metric];
      requireValue(object(outcomes) && Object.keys(outcomes).length === stratum.checks[metric].length
        && stratum.checks[metric].every(check => Object.hasOwn(outcomes, check)
          && [true, false, null].includes(outcomes[check])), 'Every independent check must be recorded; null means unverified, not success');
    }
  }
  return strata;
}

function seenBefore(set, value) {
  if (set.has(value)) return true;
  set.add(value);
  return false;
}

export function summarizeReliability(input) {
  const strata = validate(input);
  const confidence = 0.95;
  const target = 0.99;
  const alpha = (1 - confidence) / METRICS.length;
  const counts = Object.fromEntries(METRICS.map(metric => [metric, { successes: 0, trials: 0, unverified: 0 }]));
  const coverage = [...strata.values()].map(stratum => ({
    id: stratum.id, required: stratum.minimum_samples, observed: 0, duration_qualified: 0,
  }));
  const byStratum = new Map(coverage.map(item => [item.id, item]));
  const observedIds = new Set(input.samples.map(sample => sample.id));
  let missingTrials = 0;
  for (const trial of input.scheduled_trials) {
    const missing = !observedIds.has(trial.id);
    if (missing) missingTrials++;
    for (const metric of strata.get(trial.stratum).metrics) {
      counts[metric].trials++;
      if (missing) counts[metric].unverified++;
    }
  }
  for (const sample of input.samples) {
    const stratum = strata.get(sample.stratum);
    const durationQualified = sample.duration_ms >= stratum.minimum_duration_ms;
    const slice = byStratum.get(sample.stratum);
    slice.observed++;
    if (durationQualified) slice.duration_qualified++;
    for (const metric of stratum.metrics) {
      const outcomes = Object.values(sample.checks[metric]);
      if (outcomes.some(value => value === null)) counts[metric].unverified++;
      if (durationQualified && outcomes.every(value => value === true)) counts[metric].successes++;
    }
  }
  const metrics = Object.fromEntries(METRICS.map(metric => {
    const count = counts[metric];
    const bound = exactLowerBound(count.successes, count.trials, alpha);
    return [metric, {
      ...count, failures: count.trials - count.successes,
      observed_rate: count.trials === 0 ? null : count.successes / count.trials,
      lower_confidence_bound: bound, passes: count.trials > 0 && bound > target,
    }];
  }));
  const coverageComplete = coverage.every(item => item.duration_qualified >= item.required);
  const metricPass = METRICS.every(metric => metrics[metric].passes);
  return {
    schema_version: 1, suite_id: input.suite_id, runtime_build_digest: input.runtime_build_digest,
    model: input.model, sample_count: input.samples.length,
    scheduled_trial_count: input.scheduled_trials.length, missing_trials: missingTrials,
    target, family_confidence: confidence, method: 'one-sided-exact-bonferroni',
    zero_failure_trials_needed_per_metric: Math.ceil(Math.log(alpha) / Math.log(target)),
    metrics, coverage, coverage_complete: coverageComplete,
    status: coverageComplete && metricPass && missingTrials === 0 ? 'pass' : 'not_proven',
    scope: 'Supplied frozen suite only; provenance, independence and representativeness require a separate audit. No global 99% guarantee.',
  };
}

function main(args) {
  if (args.length !== 2 || args[0] !== '--input') {
    console.error('usage: node scripts/validation/agent-reliability-report.mjs --input evidence.json');
    process.exitCode = 2;
    return;
  }
  try {
    requireValue(statSync(args[1]).size <= MAX_INPUT_BYTES, 'Evidence file exceeds the input budget');
    const report = summarizeReliability(JSON.parse(readFileSync(args[1], 'utf8')));
    console.log(JSON.stringify(report, null, 2));
    process.exitCode = report.status === 'pass' ? 0 : 1;
  } catch {
    // Never forward arbitrary input, paths, parser snippets or credentials.
    console.error('agent_reliability_status=invalid_evidence');
    process.exitCode = 2;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2));
}
