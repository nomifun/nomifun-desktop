# Turn Process Terminal Status Design

## Problem

The conversation turn header currently aggregates every historical process item. A failed tool call or error tip can therefore keep the header in `failed` even after the agent recovers and produces a final assistant response. The result is a red “Failed” header for a turn that actually finished.

This aggregation is implemented in the shared React UI. Desktop builds for macOS, Linux, and Windows, as well as the Web UI, consume the same model and are affected by the same defect.

## Required display semantics

- Every closed, non-canceled turn is terminally processed, including a turn whose last diagnostic item is an error and which produced no final assistant text. Its header displays `Processed {{duration}}` (`已处理 {{duration}}` in Chinese).
- A turn whose final process evidence is canceled remains canceled, even when an earlier process item failed. Its header displays `Canceled {{duration}}` (`已取消 {{duration}}` in Chinese).
- A live turn may display processing or waiting-for-confirmation status with its running duration.
- A historical process failure remains visible inside the expanded process trace, but it never overrides a later completed terminal outcome.
- No successful terminal header uses “success” wording. Completion is expressed only as “Processed”.
- The turn header never displays failed or success wording. Failure remains diagnostic detail; the header describes whether processing is active, processed, waiting, or canceled.

## State precedence

The disclosure model owns turn-level state. Item-level state remains unchanged for diagnostic fidelity.

For a live turn, waiting-for-confirmation remains `waiting`; every other aggregate result remains `running` until the backend closes the turn.

For a closed turn, preserve `canceled` when the final process item is canceled; normalize every other aggregate result, including `failed`, to `completed`. This makes a later explicit cancellation authoritative over an earlier failure without mistaking an earlier canceled detail for the final outcome.

The duration always uses the existing disclosure interval: earliest process start through the final assistant timestamp or latest terminal process timestamp. Completed and canceled headers both include this duration.

## Scope

The fix is confined to the shared UI state model and header label mapping. Backend execution records, tool-call error classification, and expanded process-row rendering do not change.

## Verification

Regression tests cover:

- intermediate failure followed by a final assistant response → completed header;
- final cancellation, including failure followed by cancellation → canceled header;
- canceled header retains the duration placeholder;
- closed failed turn without a final assistant response → processed header while its item remains failed;
- stale running/waiting items with a final assistant response still close as completed;
- existing Nomi and ACP turn reducers remain green;
- shared UI type-check and production build pass, covering the code bundled for macOS, Linux, Windows, and Web UI.
