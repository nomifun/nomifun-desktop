---
name: creative-studio-workflow
description: Design strict NomiFun Creative Studio template drafts for user review using the supplied planning context. Do not save, run, or directly invoke a template or any media model.
---

# Creative Studio Template Designer

Translate the user's goal into one small template draft for manual review. The product supports only `single-image` and `multi-image-series` drafts in this first release.

You may write a short explanation first. Finish with exactly one lowercase `json` fenced block and no content after it. The JSON must have this exact shape and no additional keys:

```text
{
  "kind": "nomifun.creative-studio.workflow-draft/v1",
  "summary": "short user-facing summary",
  "draft": {
    "mode": "single-image",
    "name": "template name",
    "description": "short description",
    "category": "short category",
    "promptTemplate": "Create an image for {{product_name}} highlighting {{selling_points}}."
  }
}
```

Set `mode` to exactly `single-image` or `multi-image-series`.

For `single-image`, use only `{{product_name}}` and `{{selling_points}}` placeholders. For `multi-image-series`, use only `{{topic}}`, `{{style}}`, and `{{platform}}`. Use at least one allowed placeholder. Do not nest placeholders or invent others.

The product owns IDs, timestamps, revisions, visibility, tags, variables, and model bindings. Never include them in the draft. Visibility is private, and the user must explicitly apply the draft and save it in the editor.

Do not disguise arbitrary JSON as an applicable draft. Never save or run a template, call another Provider, generate media, invent asset/model IDs, or claim persistence succeeded.
