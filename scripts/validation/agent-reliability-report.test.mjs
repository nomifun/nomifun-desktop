import assert from 'node:assert/strict';
import test from 'node:test';
import { exactLowerBound, summarizeReliability } from './agent-reliability-report.mjs';

// Synthetic records test the gate, not the product. They must never be used as
// claimed live-provider acceptance evidence or written into the live corpus.
function evidence(count) {
  const build = 'a'.repeat(64);
  const suite = 'gate-unit-fixture';
  const checks = () => ({
    tools: { result_verified: true },
    execution: { terminal_verified: true, no_duplicate_effects: true },
    quality: { independent_artifact_check: true },
  });
  return {
    schema_version: 1, suite_id: suite, runtime_build_digest: build, model: 'step-3.7-flash',
    strata: [{ id: 'coding', minimum_samples: 1, minimum_duration_ms: 0,
      metrics: ['tools', 'execution', 'quality'],
      checks: Object.fromEntries(Object.entries(checks()).map(([metric, values]) => [metric, Object.keys(values)])),
    }],
    scheduled_trials: Array.from({ length: count }, (_, index) => ({ id: `trial-${index}`, stratum: 'coding' })),
    samples: Array.from({ length: count }, (_, index) => ({
      id: `trial-${index}`, session_id: `session-${index}`, suite_id: suite,
      runtime_build_digest: build, model: 'step-3.7-flash',
      stratum: 'coding', source: 'live_product', grader: 'independent_assertions',
      duration_ms: 100, checks: checks(),
    })),
  };
}

test('zero and small samples cannot establish 99% reliability', () => {
  assert.equal(exactLowerBound(0, 0), 0);
  assert.equal(exactLowerBound(0, 100), 0);
  assert.equal(summarizeReliability(evidence(0)).metrics.tools.observed_rate, null);
  for (const count of [0, 1, 10, 100, 299, 407]) {
    assert.equal(summarizeReliability(evidence(count)).status, 'not_proven');
  }
});

test('zero-failure thresholds use exact one-sided bounds and family correction', () => {
  assert.ok(exactLowerBound(298, 298) < 0.99);
  assert.ok(exactLowerBound(299, 299) > 0.99);
  const report = summarizeReliability(evidence(408));
  assert.equal(report.zero_failure_trials_needed_per_metric, 408);
  assert.equal(report.status, 'pass');
  assert.equal(report.method, 'one-sided-exact-bonferroni');
  assert.ok(report.metrics.quality.lower_confidence_bound > 0.99);
});

test('general exact bound agrees with an independently evaluated binomial tail', () => {
  const oneOfTen = exactLowerBound(1, 10);
  assert.ok(Math.abs(oneOfTen - (1 - 0.95 ** (1 / 10))) < 1e-12);
  const choose = [1, 10, 45, 120, 210, 252, 210, 120, 45, 10, 1];
  for (const successes of [2, 5, 9]) {
    const bound = exactLowerBound(successes, 10);
    let tail = 0;
    for (let k = successes; k <= 10; k++) tail += choose[k] * bound ** k * (1 - bound) ** (10 - k);
    assert.ok(Math.abs(tail - 0.05) < 1e-12);
  }
  assert.ok(exactLowerBound(990, 1000) > exactLowerBound(99, 100));
});

test('one failure stays in the denominator and does not become a passed retry', () => {
  const input = evidence(408);
  input.samples[0].checks.quality.independent_artifact_check = false;
  const report = summarizeReliability(input);
  assert.equal(report.metrics.quality.trials, 408);
  assert.equal(report.metrics.quality.successes, 407);
  assert.equal(report.metrics.quality.failures, 1);
  assert.equal(report.metrics.tools.passes, true);
  assert.equal(report.metrics.quality.passes, false);
  assert.equal(report.status, 'not_proven');
});

test('unverified checks are failures, not silently excluded observations', () => {
  const input = evidence(408);
  input.samples[0].checks.execution.terminal_verified = null;
  const report = summarizeReliability(input);
  assert.equal(report.metrics.execution.unverified, 1);
  assert.equal(report.metrics.execution.trials, 408);
  assert.equal(report.metrics.execution.successes, 407);
  assert.equal(report.status, 'not_proven');
});

test('a missing scheduled run stays in the denominator and cannot be silently discarded', () => {
  const input = evidence(1000);
  input.samples.pop();
  const report = summarizeReliability(input);
  assert.equal(report.sample_count, 999);
  assert.equal(report.scheduled_trial_count, 1000);
  assert.equal(report.missing_trials, 1);
  assert.equal(report.metrics.tools.trials, 1000);
  assert.equal(report.metrics.tools.unverified, 1);
  assert.equal(report.status, 'not_proven');
  input.scheduled_trials.pop();
  assert.equal(summarizeReliability(input).missing_trials, 0);
  input.samples[0].id = 'unscheduled';
  assert.throws(() => summarizeReliability(input));
});

test('an unexercised scenario or insufficient long-task duration blocks the gate', () => {
  const input = evidence(408);
  input.strata.push({ ...structuredClone(input.strata[0]), id: 'restart', minimum_samples: 10 });
  assert.equal(summarizeReliability(input).status, 'not_proven');
  input.strata.pop();
  input.strata[0].minimum_duration_ms = 3_600_000;
  const report = summarizeReliability(input);
  assert.equal(report.coverage[0].observed, 408);
  assert.equal(report.coverage[0].duration_qualified, 0);
  assert.equal(report.metrics.execution.successes, 0);
  assert.equal(report.status, 'not_proven');
});

test('different builds, models, suites and duplicate task episodes cannot be pooled', () => {
  for (const mutate of [
    input => { input.samples[1].runtime_build_digest = 'b'.repeat(64); },
    input => { input.samples[1].model = 'different-model'; },
    input => { input.samples[1].suite_id = 'different-suite'; },
    input => { input.samples[1].id = input.samples[0].id; },
    input => { input.samples[1].session_id = input.samples[0].session_id; },
    input => { input.samples[1].stratum = 'undeclared'; },
    input => { input.samples[1].source = 'mock'; },
    input => { input.samples[1].grader = 'model_self_report'; },
  ]) {
    const input = evidence(2);
    mutate(input);
    assert.throws(() => summarizeReliability(input));
  }
});

test('each frozen model has its own report without weakening thresholds or pooling models', () => {
  for (const model of ['step-3.7-flash', 'gpt-5.5', 'claude-sonnet-4-6', 'models/gemini-3-pro', 'Qwen/Qwen3-Coder:latest']) {
    const input = evidence(2);
    input.model = model;
    for (const sample of input.samples) sample.model = model;
    const report = summarizeReliability(input);
    assert.equal(report.model, model);
    assert.equal(report.status, 'not_proven');
    assert.equal(report.zero_failure_trials_needed_per_metric, 408);
    input.samples[1].model = `${model}-another`;
    assert.throws(() => summarizeReliability(input));
  }
  for (const model of ['', '*', ' model', 'model\n', 'x'.repeat(201), ['model'], null]) {
    const input = evidence(0); input.model = model;
    assert.throws(() => summarizeReliability(input));
  }
});

test('missing, extra and malformed assertions cannot weaken the acceptance manifest', () => {
  for (const mutate of [
    input => { delete input.samples[0].checks.quality; },
    input => { delete input.samples[0].checks.execution.no_duplicate_effects; },
    input => { input.samples[0].checks.execution.extra_check = true; },
    input => { input.samples[0].checks.quality.independent_artifact_check = 'true'; },
    input => { input.strata[0].checks.quality = []; },
    input => { input.strata[0].metrics = ['execution']; },
    input => { input.strata[0].minimum_samples = 0; },
  ]) {
    const input = evidence(1);
    mutate(input);
    assert.throws(() => summarizeReliability(input));
  }
});

test('invalid numeric inputs fail closed', () => {
  for (const values of [[-1, 10], [11, 10], [1.5, 10], [1, -1], [1, 100_001], [1, Infinity], [1, 10, 0], [1, 10, 1]]) {
    assert.throws(() => exactLowerBound(...values));
  }
});
