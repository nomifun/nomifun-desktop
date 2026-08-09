-- Backfill: remove bracketed stage directions from already-persisted robot
-- transcripts.
--
-- The robot prompt once asked the model for a leading `[emotion:name]` marker to
-- drive the OLED face. The model emitted the BARE name instead — `[winking]` —
-- so every stripper keyed on the literal `[emotion:` matched nothing and the
-- annotation was both printed in the transcript and read aloud. That design is
-- deleted, not re-syntaxed: the prompt now forbids brackets outright, and
-- `nomifun_common::stage_direction` removes a bracketed annotation of ANY syntax
-- on both live paths — `StreamRelay`'s `robot_session` gate (desktop transcript,
-- the `messages` row, search, the knowledge writeback) and
-- `sanitize_for_speech` / `sanitize_for_display` (TTS, OLED).
--
-- A prospective seam cleans nothing that is already in the database, and the
-- user is looking at that history now, so this one-shot statement cleans it. A
-- completed migration is immutable and runs exactly once, so it cannot drift
-- from the Rust policy and cannot leak into a future conversation kind.
--
-- Gated four ways so the blast radius is exactly "assistant text in a robot
-- thread": `type = 'text'`, `position = 'left'` (never a user message),
-- `conversations.extra.robot_session = 1`, and a bracket must actually occur.
-- `json_valid` / `json_type` keep a legacy or non-object `content` out;
-- `json_extract` / `json_set` own the encoding, so text containing a quote or a
-- backslash is rewritten to valid JSON rather than to a broken blob.
--
-- MATCH RULE — the Rust guard's, transcribed. Strip a bracketed run only when its
-- inner text is short (<= 24 characters), holds at least one ASCII letter, and
-- holds nothing but ASCII letters, digits, spaces, `_`, `-`, `:`. So `[winking]`,
-- `[emotion:winking]`, `[laughs]` and `[smiling softly]` go, while `[1]`,
-- `[2026]`, `[附录2]` and `[TODO 中文]` — content a human wrote — stay, and an
-- unclosed `[` survives verbatim so no row ever loses the rest of its line.
-- Deliberately vocabulary-free: the whole point of the deleted design is that a
-- model emits names nobody enumerated.
--   * `length()` counts characters, while the Rust bound is 24 BYTES. The two
--     agree on every run that can qualify, because a qualifying inner text is
--     pure ASCII; a non-ASCII one is rejected by the character-set test under
--     either measure.
--   * `【…】` is folded onto `[…]` in `probe` only to LOCATE candidates. Each
--     replacement is one character for one character, so every position computed
--     on the probe indexes the original text unchanged, and the original is what
--     gets cut. The one divergence from Rust is a mixed pair (`[winking】`),
--     which the probe accepts and Rust keeps; for a one-shot history clean that
--     is harmless either way.
--
-- SHAPE — why this is a three-phase state machine rather than one expression per
-- step. SQLite forbids the recursive table inside a subquery, so `open`, `close`
-- and the strip decision cannot be computed once in a derived table and reused;
-- written inline they would be duplicated into every output column. Instead each
-- step advances one phase and CARRIES its result in a column, which is what keeps
-- every expression below written exactly once:
--   phase 0 (`open IS NULL`)                 locate the next candidate;
--   phase 1 (`open` set, `strip IS NULL`)    judge it from the carried positions;
--   phase 2 (`strip` set)                    cut, then clear for the next round.
-- Phase 1 is also the terminator: with no candidate left (`open = 0`, or no `]`
-- after it) no phase 2 follows, and the row keeps `head || rest` intact.
--
-- A rejected candidate moves only its opening bracket into `head`, so scanning
-- resumes BEHIND it — that is what stops a kept `[1]` from hiding a real
-- `[winking]` later in the same line. `hits` counts real cuts, so a row whose
-- brackets were all content is never rewritten at all.
--
-- `messages.conversation_id` holds the conversation's TEXT business id, not the
-- `conversations.id` autoincrement surrogate; the join must use the business
-- key or it silently matches nothing.

WITH RECURSIVE peeled(message_id, head, rest, probe, open, close, strip, step, hits) AS (
    SELECT m.message_id, '',
           json_extract(m.content, '$.content'),
           replace(replace(json_extract(m.content, '$.content'), '【', '['), '】', ']'),
           CAST(NULL AS INTEGER), CAST(NULL AS INTEGER), CAST(NULL AS INTEGER),
           0, 0
      FROM messages m JOIN conversations c ON c.conversation_id = m.conversation_id
     WHERE m.type = 'text' AND m.position = 'left'
       AND json_valid(m.content) AND json_type(m.content, '$.content') = 'text'
       AND json_extract(c.extra, '$.robot_session') = 1
       AND (instr(json_extract(m.content, '$.content'), '[') > 0
            OR instr(json_extract(m.content, '$.content'), '【') > 0)
    UNION ALL
    SELECT message_id,
           -- Phase 2 cuts: `strip` is 1/0, so one expression covers both
           -- readings — drop the run (text before the bracket goes to `head`,
           -- `rest` resumes past the closer at `open + close - 1`), or keep the
           -- bracket as content (it goes to `head`, `rest` resumes behind it).
           CASE WHEN strip IS NULL THEN head
                ELSE head || substr(rest, 1, open - strip) END,
           CASE WHEN strip IS NULL THEN rest
                ELSE substr(rest, open + strip * (close - 1) + 1) END,
           CASE WHEN strip IS NULL THEN probe
                ELSE substr(probe, open + strip * (close - 1) + 1) END,
           -- Phase 0 locates: `open` is the next opening bracket, `close` the
           -- offset of the first `]` behind it. Phase 1 holds both; phase 2
           -- clears them so the next step starts a fresh round.
           CASE WHEN open IS NULL THEN instr(probe, '[')
                WHEN strip IS NULL THEN open END,
           CASE WHEN open IS NULL THEN instr(substr(probe, instr(probe, '[')), ']')
                WHEN strip IS NULL THEN close END,
           -- Phase 1 judges, from the carried positions alone: inner length is
           -- `close - 2`, and the inner text is `substr(probe, open + 1, close - 2)`.
           CASE WHEN open IS NULL THEN NULL
                WHEN strip IS NULL THEN (close - 2 <= 24
                     AND substr(probe, open + 1, close - 2) GLOB '*[A-Za-z]*'
                     AND NOT substr(probe, open + 1, close - 2) GLOB '*[^A-Za-z0-9 _:-]*')
                END,
           step + 1,
           CASE WHEN strip IS NULL THEN hits ELSE hits + strip END
      FROM peeled
     -- Phase 0 and phase 2 always advance; phase 1 advances only when it found a
     -- candidate, which is what ends the walk.
     WHERE open IS NULL OR strip IS NOT NULL OR (open > 0 AND close > 0)
)
UPDATE messages
   SET content = json_set(content, '$.content',
                          (SELECT head || rest FROM peeled s
                            WHERE s.message_id = messages.message_id
                            ORDER BY s.step DESC LIMIT 1))
 WHERE message_id IN (SELECT message_id FROM peeled WHERE hits > 0);

-- A reply that was ONLY a stage direction now holds nothing, or only the newline
-- that followed it — hence the explicit character set on `trim`, since bare
-- `trim()` strips spaces and nothing else. Hide such a row rather than leaving a
-- blank bubble in the history: exactly what `finalize` does live when the
-- middleware empties a turn (`let hidden = final_text.is_empty();`). Going
-- forward the relay never creates such a row at all — `flush_text_segment` /
-- `finalize_text_segment` return early on an empty buffer — so this only settles
-- the past.

UPDATE messages SET hidden = 1
 WHERE type = 'text' AND position = 'left' AND json_valid(content)
   AND json_type(content, '$.content') = 'text'
   AND trim(json_extract(content, '$.content'),
            ' ' || char(9) || char(10) || char(13)) = ''
   AND conversation_id IN (SELECT conversation_id FROM conversations
                            WHERE json_extract(extra, '$.robot_session') = 1);
