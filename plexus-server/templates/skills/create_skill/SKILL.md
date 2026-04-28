---
name: create_skill
description: Create a new reusable skill file in your own workspace.
always_on: false
---

# Creating Skills

When you notice a reusable pattern — a workflow you've done more than
once, a troubleshooting sequence, a set of conventions for a project —
save it as a skill.

## Location

Every skill lives at `skills/{name}/SKILL.md` inside the user's
workspace. The directory name is the skill name.

## Structure

Every SKILL.md starts with YAML frontmatter:

```
---
name: <matches the directory>
description: <one-line summary, shown in the skill index>
always_on: <true or false>
---

<instructions>
```

- `always_on: true` means the full skill content is injected into
  every system prompt. Use sparingly — consumes context budget.
- `always_on: false` means only the name+description is indexed.
  The agent reads the full file via `read_file` when needed.

## When to create one

- A workflow you've repeated 2+ times for the same user.
- A domain they care about (their codebase conventions, their
  writing style, their team's meeting rhythm).
- A troubleshooting procedure worth preserving.

## When NOT to create one

- One-off tasks.
- Things that belong in `MEMORY.md` (facts about the user) rather
  than reusable instructions.
- Work-in-progress — finish the task, then decide if a skill is
  earned.

## How to create one

```
write_file("skills/my_skill_name/SKILL.md", "---\nname: my_skill_name\ndescription: ...\nalways_on: false\n---\n\n...")
```

Use `create_dir` (implicit via `write_file` auto-creating parents).
Pick a short `snake_case` name. Keep descriptions to one line.
