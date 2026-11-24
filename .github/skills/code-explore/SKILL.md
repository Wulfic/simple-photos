---
name: code-explore
description: "Explore and understand a codebase before implementing. Use when: working in unfamiliar code, finding where a function is defined or called, looking for an existing pattern to follow, checking whether a feature already exists, or tracing callers before changing a signature."
argument-hint: "What you're looking for (e.g. 'how API routes are structured', 'where UserService is defined')"
---

# Code Explore — Understand Before You Build

Duplicating existing code and ignoring existing conventions are the two fastest ways to
make a codebase unmaintainable, and both come from skipping five minutes of reading.

## When to Use

- Any codebase you haven't touched in over a day
- Before adding a feature — does something like it already exist?
- Before creating a file — where do similar files live?
- Before changing a signature — who calls this?
- "How does this codebase do X?"

## Tool Selection — Cheapest First

Escalate only when the cheaper tool genuinely came back empty.

| Goal | Tool |
|---|---|
| Known string or symbol name | `grep_search` |
| Find files by name/path | `file_search` |
| Concept, exact name unknown | `semantic_search` |
| All usages of a symbol | `vscode_listCodeUsages` |
| Structure of an unfamiliar directory | `list_dir` |

Use `semantic_search` only when you *don't* know the exact name — if you do, `grep_search`
is faster and exact. Semantic search over a known string is a slower way to get a worse answer.

## Procedure

### Step 1 — Orient (unfamiliar codebase only)

```
list_dir("<project>")
list_dir("<project>/src")
```

Read the entry points, in this order:
- `package.json` / `pyproject.toml` / `go.mod` — deps, scripts, and how it's actually run
- `README.md` — intended usage
- The main entry file — how it wires together

### Step 2 — Check memory first

```
grep_search(query: "<area>", includePattern: ".agent/memory/*.md")
```

Cheaper than reading code, and it's where the *why* lives. If a decision already records
how this area works, start from that instead of reverse-engineering it.

### Step 3 — Find the relevant code

```
grep_search(query: "class AuthService|function handleAuth", isRegexp: true)
file_search(query: "**/auth/**")
semantic_search("authentication middleware user login")
```

Start with the narrowest query that could work, then widen. A regex that returns 200 hits
told you nothing; one that returns 3 told you where to read.

### Step 4 — Extract the pattern, don't just find the file

Read **at least two examples** before concluding you understand a convention. One example
is an anecdote; the second tells you which parts are the pattern and which were incidental.

Note specifically:
- **Naming** — files, functions, types, test files
- **Imports** — relative vs alias, barrel files
- **Errors** — throw vs return, Result type vs try/catch, custom error classes
- **Logging** — which logger, what levels, what gets included
- **Async** — promises, async/await, callbacks
- **Tests** — co-located or separate, what's mocked
- **Config** — env vars, config objects, defaults

### Step 5 — Find every caller before changing a signature

```
vscode_listCodeUsages(symbol: "functionName", filePath: "...", lineContent: "export function functionName")
grep_search(query: "functionName", isRegexp: false)
```

Run **both**. `vscode_listCodeUsages` understands types but misses anything the language
server can't resolve; `grep_search` catches the rest, including comments and docs that
reference the old name.

**Every caller gets updated or explicitly confirmed compatible.** Also check for dynamic
callers a symbol search won't find: string-keyed dispatch, DI containers, reflection,
config-driven routing. Those are exactly the ones that fail in production rather than at
compile time.

### Step 6 — Summarise before you build

State plainly:
- Where similar code lives
- Which pattern you'll follow, and the file you're copying it from
- What to avoid, and why
- Anything surprising

If a convention was non-obvious or a trap cost you time, append it via `save-memory` — the
next session shouldn't have to rediscover it.

## Rules

- **Never implement without exploring.** Duplication you caused is a bug you caused.
- **Follow existing patterns.** Even if you'd do it differently — match first, refactor in
  a separate change. A codebase with three competing conventions is worse than one with a
  single mediocre convention.
- **Two examples minimum** before you trust a pattern.
- **If it already exists, use it.** Don't rebuild it because you didn't like the name.
- **Budget ~10 minutes.** If you still can't find it, stop and say what you searched for
  and where — don't quietly assume it doesn't exist and build a duplicate.
