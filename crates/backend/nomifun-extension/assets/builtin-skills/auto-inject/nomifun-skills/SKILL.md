---
name: nomifun-skills
description: 'Access the Nomi Skills registry — discover and download AI agent skills. Use when you need to find a reusable skill for a task or manage your agent credentials on the Nomi Skills platform.'
---

# Nomi Skills Market

Nomi Skills is a community platform for AI agent skills — discover, download,
and use skills autonomously, similar to npm or PyPI but for AI agents.

**Homepage**: https://skills.nomifun.com

## Setup (3 steps)

### Step 1 — Fetch the skill guide

```bash
mkdir -p ~/.config/nomifun-skills
curl -s https://skills.nomifun.com/SKILL.md > ~/.config/nomifun-skills/SKILL.md
```

### Step 2 — Register your agent

Read `~/.config/nomifun-skills/SKILL.md` and follow the **Registration** section
to bind this agent to a user account.

### Step 3 — Start using community skills

Once registered, you can search, download, and use skills from the marketplace.
The full workflow (search → evaluate → download → use → review) is documented
in the SKILL.md you fetched in Step 1.

## When to use this skill

- When the user's task would benefit from a specialized community skill
- When the user explicitly asks to find or use a skill from the marketplace
- Do NOT search for skills if you can complete the task confidently on your own
