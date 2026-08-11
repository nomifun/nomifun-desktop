-- Older desktop builds persisted several named presets as platform='custom'.
-- That erased the provider identity needed for task-specific routing, so an
-- xAI image model, for example, was later sent through the OpenAI multipart
-- adapter. Reclassify only exact official preset roots; genuine custom
-- gateways and new-api rows are intentionally untouched.

UPDATE providers
SET platform = CASE
    WHEN lower(rtrim(base_url, '/')) IN (
        'https://api.openai.com',
        'https://api.openai.com/v1'
    ) THEN 'openai'
    WHEN lower(rtrim(base_url, '/')) = 'https://api.novita.ai/openai/v1' THEN 'novita'
    WHEN lower(rtrim(base_url, '/')) = 'https://openrouter.ai/api/v1' THEN 'openrouter'
    WHEN lower(rtrim(base_url, '/')) = 'https://api.x.ai/v1' THEN 'xai'
    WHEN lower(rtrim(base_url, '/')) = 'https://api.poe.com/v1' THEN 'poe'
    WHEN lower(rtrim(base_url, '/')) IN (
        'https://api.ppio.com/openai/v1',
        'https://api.ppinfra.com/v3/openai',
        'https://api.ppinfra.com/v3/openai/v1'
    ) THEN 'ppio'
    WHEN lower(rtrim(base_url, '/')) = 'https://api-inference.modelscope.cn/v1' THEN 'modelscope'
    WHEN lower(rtrim(base_url, '/')) = 'https://cloud.infini-ai.com/maas/v1' THEN 'infiniai'
    WHEN lower(rtrim(base_url, '/')) IN (
        'https://wishub-x6.ctyun.cn/v1',
        'https://wishub-x1.ctyun.cn',
        'https://wishub-x1.ctyun.cn/v1'
    ) THEN 'ctyun'
    ELSE platform
END
WHERE platform = 'custom';

-- The two retired roots have direct current successors. Normalize only rows
-- that have just been identified as those products, preserving arbitrary
-- user-entered custom URLs byte-for-byte.
UPDATE providers
SET base_url = 'https://api.ppio.com/openai/v1'
WHERE platform = 'ppio'
  AND lower(rtrim(base_url, '/')) IN (
      'https://api.ppinfra.com/v3/openai',
      'https://api.ppinfra.com/v3/openai/v1'
  );

UPDATE providers
SET base_url = 'https://wishub-x6.ctyun.cn/v1'
WHERE platform = 'ctyun'
  AND lower(rtrim(base_url, '/')) IN (
      'https://wishub-x1.ctyun.cn',
      'https://wishub-x1.ctyun.cn/v1'
  );
