---
name: code-review
description: "Thorough, skeptical code review. Use when: reviewing a PR, auditing a file before merge, checking for security or OWASP issues, hunting missing error handling or logging, or self-reviewing your own diff before committing."
argument-hint: "File path(s) or feature area to review"
---

# Code Review — Assume It's Broken

Review like someone who has seen every mistake and has no incentive to be polite. Every
real issue gets called out with a file and line. **Don't pad the report with praise —
silence means "no objection", not "good work".**

This applies to **your own diff** too, and that's the harder case: you already believe it
works, which is exactly the state in which people miss things. Read it as if rejecting it.

## When to Use

- Before committing anything non-trivial (self-review)
- Before opening or approving a PR
- Security or OWASP pass
- "Does this actually do what it claims?"

## Procedure

### Step 1 — Read it properly

```
read_file(...)        // the full file, not the diff hunks alone
grep_search(...)      // find every caller of what changed
```

A diff without surrounding context hides the bugs that matter. Specifically:

- **Trace the data end to end** — where does input enter, what validates it, where does it land?
- **Read every caller of a changed signature.** A compatible-looking change that breaks
  one of six call sites is the classic miss.
- **Note what is absent.** Missing error handling, missing logging, missing tests, and
  missing validation don't appear in a diff — you have to actively look for the gap.

### Step 2 — Work the checklist

Don't freestyle. Freestyle review finds whatever you happened to think of that day.

**Correctness**
- [ ] Off-by-one, boundaries, empty collections, null/undefined
- [ ] Every `async` awaited; no floating promises
- [ ] Errors handled at the right level — not swallowed, not blindly rethrown
- [ ] Early returns leave state consistent
- [ ] Concurrency: shared mutable state, race conditions, non-atomic read-modify-write

**Security (OWASP)**
- [ ] Injection — SQL/command/LDAP; parameterised queries, never string concatenation
- [ ] AuthN/AuthZ enforced **server-side**, on every path, not just the happy one
- [ ] No secrets, tokens, or keys in code, logs, or error messages
- [ ] Input validated at the trust boundary, not deep inside
- [ ] No PII in logs; no sensitive data in URLs
- [ ] Dependencies — anything new, unmaintained, or on a known-bad version?
- [ ] Output encoded for its sink (HTML, SQL, shell, JSON)

**Destruction & data safety** — the findings here are the expensive ones
- [ ] Every delete/overwrite/reset/migrate path found — `grep` for `delete`, `drop`,
      `truncate`, `rm`, `unlink`, `rmdir`, `overwrite`, `reset`, `purge`, `prune`
- [ ] **No unbounded filter.** A delete or update that runs with an empty/null filter and
      treats it as "match everything" is CRITICAL, every time
- [ ] Destructive queries parameterised — never a filter built by string concatenation
- [ ] Path operations resolve, then assert the target is inside the intended root; `..`,
      symlinks, and absolute paths from input are rejected
- [ ] Confirmation exists and states **what and how many**; dry-run is the default and
      `--force` the opt-in; non-interactive contexts require an explicit flag
- [ ] Reversible — soft delete where the record is referenced; every migration `up` has a
      `down` that has actually been run
- [ ] Backup happens **and is verified** before the destructive step, not after
- [ ] Fails closed — an indeterminate safety check refuses rather than proceeding
- [ ] Blast radius capped: an operation affecting more than N rows/files stops for an override
- [ ] Logs before and after, with the affected count and the recovery path
- [ ] **A test feeds the bad input and asserts nothing was destroyed** — not just that the
      happy path deletes correctly

**Operability**
- [ ] Every error path logs something actionable, with context — not `log.error(e)` alone
- [ ] Log levels correct; no logging of secrets or full request bodies
- [ ] Failures are diagnosable from logs alone, without a debugger

**Tests**
- [ ] Tests exist and would actually fail if the change were reverted
- [ ] Edge cases covered, not just the happy path
- [ ] No test asserting current-but-wrong behaviour

**Structure**
- [ ] Follows patterns already in this codebase (verify with `code-explore`)
- [ ] Names say what things are; no `data2`, `tmp`, `handleThing2`
- [ ] Not duplicating something that already exists
- [ ] Not over-engineered for requirements that don't exist yet

### Step 3 — Check it against recorded decisions

```
grep_search(query: "<area>", includePattern: ".agent/memory/*.md")
```

Code contradicting a recorded decision is a finding — either the code is wrong, or the
decision is stale and needs a new entry. Don't let them silently diverge; that's how a
decisions file becomes fiction.

### Step 4 — Report by severity

Every finding needs **file, line, and what actually goes wrong.** "This could be better"
is not a finding — it's noise that dilutes the real ones.

**CRITICAL — blocks merge**
Security holes, data loss or corruption, auth bypass, secrets in code, crashes on a normal
path. **Any unguarded destructive path lands here** — an empty filter treated as "match
everything", a delete with no confirmation, a migration with no `down`, a path delete that
doesn't check its root. These do not get downgraded because the happy path works.

**HIGH — fix in this change**
Missing validation at a boundary, no logging on an error path, a logic bug at an edge
case, missing tests on critical paths, an unhandled failure mode.

**MEDIUM — ticket it**
Confusing structure, poor naming, unnecessary complexity, missing docs on non-obvious logic.

**LOW — optional**
Style, minor refactors.

State the failure concretely: *"`parseConfig` at [config.ts:47](config.ts#L47) throws on an
empty file; `loadAll` doesn't catch it, so a missing config crashes startup with no log."*
That's actionable. "Error handling could be improved" is not.

### Step 5 — Record anything structural

If the review reveals a pattern worth enforcing, or a trap likely to recur, append it via
`save-memory`. One-off nits don't belong in memory; recurring classes of mistake do.

## Rules

- **Every finding cites file and line.**
- **"Looks fine" is not a review** unless you actually read the code.
- **No tests → automatic HIGH.**
- **No logging on an error path → automatic HIGH.**
- **An unguarded destructive path → automatic CRITICAL.** See `guard-destructive`.
- **A guard with no test proving it blocks → automatic HIGH.** Untested guards get deleted
  by the next person as dead code, and nothing fails.
- **A suppression** (`@ts-ignore`, `as any`, empty `catch`, `eslint-disable`) **→ automatic
  HIGH** unless there's a written justification next to it.
- Don't invent problems to look thorough. A short review of clean code is a fine outcome —
  say what you checked, so the scope of that "clean" is clear.
