# Realtime voice architecture

Voice owns a separate media and interaction entry point; work runs through the single canonical AgentSession. Mobile is a separate device-media client. Desktop retains provider connections, long-lived credentials and work authority. This does not change the desktop renderer's 880×600 minimum viewport.

## Ownership

| Layer | Responsibility and source |
| --- | --- |
| Public contracts | [nomifun-voice-contracts](../../crates/backend/nomifun-voice-contracts/src/lib.rs): media, profiles, work interaction and API types |
| Media core | [nomifun-voice-core](../../crates/backend/nomifun-voice-core/src/lib.rs): provider-independent ports, sessions and media lifecycle |
| Product owner | [nomifun-voice](../../crates/backend/nomifun-voice/src/lib.rs): endpoints, VoiceJournal, sources, pending input and work bridge |
| Provider wire | [model-invoke/voice](../../crates/backend/nomifun-model-invoke/src/voice/mod.rs): registered protocols, authentication, connections and provider events |
| Application composition | [mobile_voice_host.rs](../../crates/backend/nomifun-app/src/router/mobile_voice_host.rs), [voice_work_host.rs](../../crates/backend/nomifun-app/src/router/voice_work_host.rs): authentication, registry and canonical work admission |

The registry explicitly registers production adapter identities, currently `stepfun.realtime_s2s` and `openai.gpt_live`. Models, protocols and connections are explicit selections. An adapter, audio I/O or successful handshake does not prove account access, semantic duplex behavior or listening experience. Additional adapters depend on public ports/contracts without receiving work authority.

## Media lifecycle

Input and output retain exact voice/session identity, activation epoch, generation, segment revision and actual audio format. Device, JS and server queues are bounded by audio duration and wall age. The same input frame returns its permit only on exact `InputAdmitted` or `InputReleased`; release does not prove model admission. Late acknowledgements, old generations and revoked leases cannot revive capture or playback.

Flushing must actually remove the old generation. `played` comes from the real sink clock and preserves estimated, unknown and uncertain tails. Scheduling, software dequeue and completion callbacks do not prove listening. Separate device contexts do not form one continuous consumption clock. Capture pause, playback stop, voice closure and work cancellation are distinct actions.

Backgrounding, interruption, routing and permission changes revoke the media lease. Foregrounding never automatically opens the microphone. Closure retains ownership until real cleanup and join; uncertain cleanup is reported rather than inferred from listener removal. The Mobile repository owns device-SDK implementation and build evidence; this repository no longer retains its construction prompts, old APK hashes or historical assertion counts.

## Sources and canonical work

VoiceJournal stores transcripts, source revisions, media consumption and pending input while referencing canonical work facts. It is no second Agent transcript, authorization ledger or completion ledger. Work state and speakable content come from typed Event/Payload and exact Turn receipts; reasoning and raw tool content are not directly spoken.

Only explicitly voice-started Turns enable immediate correction. Superseding a model step requires proof of cancellation and join for the original model operation. Admitted tools settle first; unknown effects cannot replay. Typed history may exclude superseded, unadmitted generation while preserving audit facts. Ordinary text input does not enable this policy.

Relative cancellation and correction must prove transcript timing, source revision and the work target at that time. Otherwise an authenticated user confirms the exact target, without borrowing the latest task at receipt time. New model-call IDs cannot re-admit an already committed source and intent. Voice closure, media failure and journal unavailability do not change existing text-work results.

After restart, an explicit user voice start performs lazy reconciliation. Confirmed unadmitted Queued input preserves its operation key, policy and binding/context-floor fences. Unknown Dispatched input queries the original canonical receipt without dispatching again. See [Agent Session](agent-session.md).

## Physical robot entry

A physical robot uses the companion's primary Agent and an independent device Session through [robot_wiring.rs](../../crates/backend/nomifun-app/src/robot_wiring.rs) and [robot session](../../crates/backend/nomifun-robot/src/session.rs). Mobile media behavior does not establish physical robot capabilities.

Early public progress after actual tool admission uses the typed `PublicOutputCommitted` boundary, with exact accepted root, model step, part count, digest and canonical commit identity. Final answers require the original `message/completed` integrity proof plus actual successful Host `turn/completed` and the canonical `Answered` message references; their cursor includes the real terminal permission. `CompletionDelivered` is still a proposal which the Host may narrow to a pause, so it is never an early speech permit. Raw tokens, reasoning, tool bodies, retractable drafts and discarded output are never spoken. Publishing a prefix does not close a growing aggregate message or replace the final outcome reducer.

Direct device clarification uses the normal typed `request_user_input` path. Questions and answers retain exact Turn and question identity; recording captures the presented question before recognition. Slow ASR, successor questions and replacement connections cannot invent a new answer relation. Click-required approvals retain their authority limits.

A committed child Turn asking for missing human input is delivered under the `automatic` decision policy as well; that policy cannot supply the human answer. The existing approval/StopTurn path still requires `AskUser`. Presentation facts are recorded after the original text enters the writer, while a queue acknowledgement grants no answer authority. Every subsequent frame rechecks the original pending question. Ordinary request media also checks the exact accepted input and Turn: UI cancellation revokes new frames, while normal Completed results retain their final delivery permission.

Ordinary tools execute in the receiving device Session. Optional background work reports directly to the original request without a mandatory result-model Turn. Success events advance delivery; durable rereads repair lost notifications or restart state.

SpeechServices explicitly distinguishes incremental ASR, streaming TTS and Batch. Streaming input sends audio during recording; streaming synthesis yields actual provider audio before response completion. Framing a fully synthesized artifact remains Batch. Playback retains pacing, bounded queues, backpressure and generation isolation. Stop playback, amend input, cancel work and start a new request have distinct targets.

The explicit `deepgram.listen_stream` protocol sends PCM over WebSocket and requires confirmed finalization after `CloseStream`. `openai.audio_speech` pulls real HTTP PCM bytes at 24 kHz, signed 16-bit little-endian mono. Other batch adapters remain Batch; transcript SSE does not imply incremental audio input. See the provider contracts for [Deepgram streaming](https://developers.deepgram.com/reference/speech-to-text/listen-streaming), [CloseStream](https://developers.deepgram.com/docs/close-stream), and [OpenAI speech](https://developers.openai.com/api/docs/guides/text-to-speech). Local controlled HTTP/WS tests do not prove real accounts, network buffering or audible first output. The pacer's own queue holds 32 audio frames of 60 ms; writer admission and physical playback remain separate facts.

Disconnect, connection replacement, rebinding and context clear revoke the original delivery range. Late media, transcripts and task results cannot enter successor requests or other connections. Media, models, device actions and background execution retain separate budgets. See the [unification acceptance record](../validation/companion-unification-2026-10-18.zh.md) for tested layers and outstanding provider, hardware and installation evidence.

## Validation boundary

Focused checks cover public ports, media budgets, late events, source retraction, journal recovery and canonical correction. Actual account permissions, both providers, LAN/TLS, Android/iOS/H5 device lifecycle, acoustics and cleanup require their respective environments. Types, simulated devices, static packages and source implementation cannot establish these results. Historical execution records remain in Git; this page makes no unverified device-acceptance claim.

Start with [core port tests](../../crates/backend/nomifun-voice-core/tests/port_contract.rs), [service tests](../../crates/backend/nomifun-voice/src/service/tests.rs), [provider contract tests](../../crates/backend/nomifun-model-invoke/src/voice/contract_tests.rs) and [runtime correction tests](../../crates/backend/nomifun-app/src/router/voice_runtime_correction_tests.rs).
