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

## Validation boundary

Focused checks cover public ports, media budgets, late events, source retraction, journal recovery and canonical correction. Actual account permissions, both providers, LAN/TLS, Android/iOS/H5 device lifecycle, acoustics and cleanup require their respective environments. Types, simulated devices, static packages and source implementation cannot establish these results. Historical execution records remain in Git; this page makes no unverified device-acceptance claim.

Start with [core port tests](../../crates/backend/nomifun-voice-core/tests/port_contract.rs), [service tests](../../crates/backend/nomifun-voice/src/service/tests.rs), [provider contract tests](../../crates/backend/nomifun-model-invoke/src/voice/contract_tests.rs) and [runtime correction tests](../../crates/backend/nomifun-app/src/router/voice_runtime_correction_tests.rs).
