# Copilot Instructions

**Read [`AGENTS.md`](../AGENTS.md) at the repo root — it is the authoritative version of
these rules.** This file is the condensed set that applies even if you never open it.

## Rule zero — destructive actions

An action is destructive if **a mistake would lose state that isn't reproducible from
what's on disk and committed**: `rm -rf`, `Remove-Item -Recurse`, overwriting a file you
never read, `git reset --hard` / `clean -fd` / `push --force` / `branch -D` / `rebase`,
`DROP` / `TRUNCATE` / `DELETE` with no `WHERE`, migration `down`, `docker volume rm` /
`compose down -v` / `system prune`, `terraform apply` or `destroy`, publishing, releasing,
or closing anything other people can see.

**Five gates, all five, in order, before it runs:**

1. **Resolve the target** — print the variable, expand the glob, `ls` the directory. You
   destroy what you just read, never the pattern you typed. An unset variable turns a
   scoped delete into a root delete.
2. **Prove it's recoverable** — name the commit, the verified backup, the snapshot. If the
   honest answer is *nowhere*, make the copy first.
3. **Dry-run it** — `git clean -nd`, `terraform plan`, `--dry-run`. No dry-run in the tool?
   Build one: `SELECT` before `DELETE`, `COUNT(*)` before `TRUNCATE`. Read the output.
4. **Narrow the scope** — the file not the directory, one named branch not `-fd`,
   `WHERE id = @id` not the table. **Never inside a `&&` chain or a loop.**
5. **Write the undo, then confirm** — quote the exact command and blast radius to the user.
   Can't write the reversing command? You have a hope, not a plan.

**Absolute stops, needing explicit in-the-moment instruction:** `rm -rf` on `/` `~` a drive
root or the repo root · `push --force` to a shared branch · `reset --hard` or `clean -fd`
with uncommitted work · dropping a database this session didn't create · `docker volume rm`
· `terraform destroy` · deleting `.agent/memory/`, `.env`, or credentials · deleting a test
or using `--no-verify` · rewriting work that isn't yours.

**"I assumed that's what you meant" is not authorization.** Ambiguous instruction plus
irreversible action means stop and ask. Approval for one destructive action never carries
to the next.

**Code you write that destroys gets the same treatment:** `--dry-run` is the default and
`--force` the opt-in; the confirmation states what *and how many*; an empty filter is
refused, never treated as "match everything"; back up and verify before destroying; log
before and after with the recovery path; and **a test feeds the bad input and asserts
nothing was destroyed** — a guard with no test gets refactored away as dead code.

There is no permission layer in Copilot. This section is the only guard. Full procedure:
[`guard-destructive`](skills/guard-destructive/SKILL.md).

## Before the first edit

1. Read `.agent/memory/state.md` — it says where the last session stopped.
2. Grep `.agent/memory/decisions.md` and `gotchas.md` for the area you're touching.
   **Check whether your approach was already tried and rejected before proposing it.**
3. Write the plan into `TODO.md`. A plan that lives only in chat did not happen.
4. Find the existing pattern in the codebase and follow it. Don't invent a second way
   to do something this repo already does.

## While working

- **Small diffs, verified often.** One logical unit → build → next. Never stack ten
  edits and then build.
- **Every loop has a budget of 3.** Three failed fixes for the same build error, test,
  or exception means stop, write the failure and all three attempts to
  `.agent/memory/gotchas.md`, and report to the user. Reporting a documented blocker is
  a valid outcome. A fourth guess is not.
- **Two failed fixes means the diagnosis is wrong, not the fix.** Re-diagnose from
  scratch instead of patching again.
- **Never widen the fix to make a symptom disappear.** Deleting the failing test, adding
  `@ts-ignore` / `as any` / `# type: ignore`, or an empty `catch` is not a fix.
- Update `.agent/memory/state.md` at each phase boundary, not just at the end — if
  context gets truncated, that file is how the next turn recovers.

## Before saying it's done

- [ ] Build exits 0, zero diagnostics, lint and typecheck clean
- [ ] A test exists that fails if the change is reverted
- [ ] Full suite green
- [ ] Every error path logs something useful
- [ ] Every destructive path is guarded, and a test proves the guard blocks
- [ ] Every destructive step taken had a verified rollback, written into `TODO.md`
- [ ] `TODO.md` matches reality
- [ ] `.agent/memory/` updated if anything non-obvious was decided or discovered
- [ ] Diff self-reviewed — no debug code, no commented-out blocks, no secrets

## Tone

Direct. Assume code is broken until a test proves otherwise. Don't pad reports with
praise — silence means "no objection". When something is wrong, say so plainly and fix it.

## Skills

Detailed procedures live in [`.github/skills/`](skills/). Invoke with `/skill-name`:
`guard-destructive`, `recall-session`, `plan-work`, `code-explore`, `research-docs`,
`build-run`, `debug-errors`, `test-iterate`, `code-review`, `save-memory`, `git-github`.

**Prefer native tools** — file editing, search, terminal, git, and web fetch are all
built in. Only two MCP servers exist: `github` (fall back to the `gh` CLI if it's
unavailable) and `context7` (live library docs).

> `.github/skills/` and `.claude/skills/` are hand-synced copies. Edit a skill in both.
