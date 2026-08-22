-- Declare each chat model's output-token ceiling next to its context window.
-- NULL is meaningful: serializers that support omission let the provider choose.
ALTER TABLE provider_model_capabilities
    ADD COLUMN output_limit INTEGER
    CHECK (output_limit IS NULL OR output_limit > 0);

-- Output ceilings are typed capability data now. Remove every legacy request-body
-- spelling before the resolver starts rejecting these no-op provider parameters.
UPDATE provider_model_capabilities
   SET provider_params = json_remove(
       provider_params,
       '$.max_tokens',
       '$.max_completion_tokens',
       '$.maxOutputTokens',
       '$.max_output_tokens',
       '$.generationConfig.maxOutputTokens'
   )
 WHERE task = 'chat';

-- A compatible OpenAI endpoint may have named a different top-level field via
-- max_tokens_field. json_quote turns that field name into a safe quoted JSON
-- path component (for example, $."max.new.tokens"). Keep max_tokens_field itself:
-- it still selects the typed serializer field; only its old untyped value goes.
UPDATE provider_model_capabilities
   SET provider_params = json_remove(
       provider_params,
       '$.' || json_quote(json_extract(provider_params, '$.max_tokens_field'))
   )
 WHERE task = 'chat'
   AND json_type(provider_params, '$.max_tokens_field') = 'text'
   AND trim(json_extract(provider_params, '$.max_tokens_field')) <> '';

-- Anthropic Messages requires max_tokens on the wire. Preserve the effective
-- 8192 value the desktop agent sent before this migration, but move it into the
-- new editable capability field. OpenAI-compatible and Gemini rows stay NULL.
UPDATE provider_model_capabilities
   SET output_limit = 8192
 WHERE task = 'chat'
   AND protocol IN ('anthropic.messages', 'bedrock.anthropic_messages');
