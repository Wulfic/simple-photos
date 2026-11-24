---
name: research-docs
description: "Pull live, up-to-date library and API documentation. Use when: asking how to use any external package, framework, or API; before implementing against a third-party SDK; when training data may be stale (anything released after 2023); looking up config options, migration guides, or changelogs. Uses context7 MCP."
argument-hint: "Library name and what you need to know (e.g. 'drizzle-orm query builder')"
---

# Research Docs via context7

Training data goes stale. context7 pulls live documentation. Always use it before answering questions about external libraries — never rely on memory alone.

## When to Use

- "How do I configure X in library Y?"
- "What's the correct API for Z in version N?"
- Anything involving a package that may have changed since 2023
- Migration guides (v1 → v2, etc.)
- Before implementing a new integration

## Tools

```
mcp_wulfnet-conte2_context7_get_tool_schema(tool_name)   # schema lookup
mcp_wulfnet-conte2_context7_invoke_tool(tool_name, tool_input)
```

Available tools (from schema): `resolve-library-id`, `query-docs`

## Procedure

### Step 1 — Resolve the library ID

```json
tool_name: "resolve-library-id"
tool_input: {
  "query": "<plain-english library name or description>",
  "libraryName": "<npm/pypi/etc package name if known>"
}
```

Returns a `libraryId`. Use the exact value in step 2.

### Step 2 — Pull the docs

```json
tool_name: "query-docs"
tool_input: {
  "libraryId": "<id from step 1>",
  "query": "<specific thing you need to know>"
}
```

### Step 3 — Apply and cite

- Quote or summarize the relevant section in your response
- If the docs contradict your training data, **trust the docs**
- If the docs are incomplete, fall back to native web search, then to reading the
  library's source in `node_modules/` or the vendor directory — the installed source is
  the most authoritative answer available, because it's the version actually running

## Rules

- Always run this before answering any "how do I use library X" question
- **Never invent API signatures from memory** for libraries you haven't confirmed. A
  plausible-looking wrong signature costs more than the lookup would have.
- Pin what you learn to a **version** — "this is the v3 API" — since the next reader may
  be on a different one
- If `resolve-library-id` returns nothing useful, use native web search, or read the
  installed source directly
- If the live docs contradict what you expected, that's a `gotchas.md` entry — use
  `save-memory`. Surprising API behaviour will surprise the next session too.
