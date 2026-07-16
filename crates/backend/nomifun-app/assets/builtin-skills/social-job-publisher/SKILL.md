---
name: social-job-publisher
description: Turn a hiring brief into a complete job description, platform-specific recruiting copy, visual briefs, and a safely confirmed publishing flow. Use when drafting or publishing recruitment campaigns for X, Xiaohongshu, LinkedIn, or similar social and hiring platforms.
---

# Social Job Publisher

Produce a consistent recruiting campaign from a rough hiring request. Draft first, adapt for each destination, and never submit content to a third-party platform without the user's explicit final confirmation.

## Intake

Extract the following from the request:

- Role title and level
- Company or team
- Location and remote, hybrid, or on-site policy
- Employment type
- Responsibilities and requirements
- Compensation, when provided
- Application method
- Target platforms

Ask only for information that materially changes the result. When details are missing and the user did not request immediate publishing, make conservative assumptions and label them for review. Require a destination platform and application method before publishing.

## Workflow

1. Draft one canonical job description.
2. Adapt the message for each requested platform.
3. Prepare image copy or visual-generation briefs when visuals are useful.
4. Validate claims, links, contact details, platform limits, and destination account.
5. Show the exact final copy, attachments, destination, and intended action.
6. Wait for explicit final confirmation immediately before any external submission.
7. Publish through an available connector or browser flow, then verify the resulting post or report the failure accurately.

## Canonical Job Description

Include:

- Role title
- Two or three sentences about the team and role
- Location and employment type
- Three to six responsibilities
- Three to six requirements
- Optional nice-to-haves
- Compensation only when known or approved
- Clear application method

Avoid invented company facts, discriminatory requirements, inflated promises, and unsupported compensation claims. Keep required qualifications distinct from preferences.

## Platform Adaptation

- **X**: Lead with the role and strongest reason to care. Keep within the platform's current post limit and use a small number of relevant hashtags.
- **Xiaohongshu**: Write a concise title, readable short paragraphs, a warm but credible tone, and three to five relevant topics.
- **LinkedIn**: Use a professional opening, scannable bullets, and a direct application call to action.
- **Hiring platforms**: Map the canonical description into the site's structured fields without dropping required facts.

When platform rules or limits may have changed, verify them before finalizing. If the user asks for only one platform, produce only that version.

## Visuals

When visuals are requested, prepare:

- A cover: role title, short value proposition, and company or team name.
- A detail image: key responsibilities, requirements, and application method.

Use an available image-generation capability or provide production-ready visual briefs. Do not fabricate a company logo or brand identity. Check all text in generated images before publishing.

## Publishing Safety

- Treat drafting, opening a composer, and submitting a post as separate actions.
- Never interpret a general request to prepare content as permission to publish it.
- Immediately before submission, present the final destination, account, text, links, and attachments.
- Require an explicit confirmation such as “publish this version” for that exact payload.
- If the payload changes after confirmation, ask again.
- Prefer a dedicated platform connector when available; otherwise use a visible browser flow.
- Stop at a draft or export when authentication, tooling, or confirmation is unavailable.

For X or Xiaohongshu, use the dedicated `x-recruiter` or `xiaohongshu-recruiter` skill when it is available and selected. This skill remains the campaign-level source of truth for the canonical job description and cross-platform consistency.
