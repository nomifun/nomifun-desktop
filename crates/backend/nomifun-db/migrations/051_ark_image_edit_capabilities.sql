-- Ark Seedream uses one /images/generations endpoint for both text-only image
-- generation and reference-image editing. Before protocol revision 51,
-- ark.images rejected the image_edit task at validation time, so an existing
-- generation-only row cannot mean that the user deliberately declined the
-- paired capability: the application gave them no valid way to declare it.

-- The capability graph changed. Bump each affected provider exactly once so
-- durable async handles and cached resolved calls cannot cross the upgrade.
UPDATE providers
   SET config_revision = config_revision + 1,
       updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
 WHERE EXISTS (
           SELECT 1
             FROM provider_model_capabilities generation
            WHERE generation.provider_id = providers.provider_id
              AND generation.task = 'image_generation'
              AND generation.protocol = 'ark.images'
              AND NOT EXISTS (
                    SELECT 1
                      FROM provider_model_capabilities edit
                     WHERE edit.provider_id = generation.provider_id
                       AND edit.model = generation.model
                       AND edit.task = 'image_edit'
              )
       );

-- The two tasks share transport, auth, endpoint, and provider-native defaults.
-- Health is intentionally reset: a successful T2I check does not prove that
-- the backing model/opaque endpoint accepts image input.
INSERT INTO provider_model_capabilities (
    provider_id,
    model,
    task,
    traits,
    protocol,
    connection_role,
    base_url_override,
    endpoint,
    poll_endpoint,
    content_endpoint,
    realtime_endpoint,
    allow_cross_origin_credentials,
    provider_params,
    context_limit,
    output_limit,
    health,
    health_checked_at,
    created_at,
    updated_at
)
SELECT
    generation.provider_id,
    generation.model,
    'image_edit',
    '[]',
    generation.protocol,
    generation.connection_role,
    generation.base_url_override,
    generation.endpoint,
    generation.poll_endpoint,
    generation.content_endpoint,
    generation.realtime_endpoint,
    generation.allow_cross_origin_credentials,
    generation.provider_params,
    NULL,
    NULL,
    NULL,
    NULL,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
  FROM provider_model_capabilities generation
 WHERE generation.task = 'image_generation'
   AND generation.protocol = 'ark.images'
   AND NOT EXISTS (
       SELECT 1
         FROM provider_model_capabilities edit
        WHERE edit.provider_id = generation.provider_id
          AND edit.model = generation.model
          AND edit.task = 'image_edit'
   );
