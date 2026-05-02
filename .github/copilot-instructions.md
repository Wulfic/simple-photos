# GitHub Copilot Instructions — simple-photos

These instructions apply to every Copilot interaction in this repository.

---

## 1 — Always Plan Before Acting

Before writing or modifying any code:

1. **Write a plan** listing every file, function, route, and migration that will be touched.
2. **Run impact analysis** (`gitnexus_impact`) on every symbol in the plan. Report the blast radius to the user.
3. **Warn immediately** if any symbol returns HIGH or CRITICAL risk — do not proceed without explicit user approval.
4. **Track progress** with `manage_todo_list`: mark one item in-progress at a time, mark it completed immediately when done.

Never begin implementation without a written, user-approved plan.

---

## 2 — Testing: DDT and E2E First

### Data-Driven Tests (DDT / parametrized)
- Every code path that accepts variable input **must** be covered by a `pytest.mark.parametrize` table.
- Pattern: one named constant table per logical group (e.g. `BRIGHTNESS_CASES`). Each row is a `pytest.param(...)` with a descriptive `id=` string. Shared test logic; only inputs vary per row.
- Reference files: `tests/test_35_edit_save_ddt.py`, `tests/test_38_edit_dimensions_ddt.py`.
- Boundary values, minimums, maximums, and invalid inputs must each be a distinct row.

### End-to-End (E2E) Tests
- Every user-facing feature **must** have at least one E2E test exercising the full request → server → storage → response cycle via `APIClient` (`tests/helpers.py`).
- New test files use the next sequential number: find the highest `test_NN_` prefix and increment by one.
- E2E tests run against the real server (`conftest.py` fixture) — never mock internals.
- After implementing a feature, run `pytest tests/test_NN_*.py -v` and confirm all pass before closing the task.

### General rules
- Do not remove or weaken existing assertions.
- `helpers.py` is shared — any edit there requires running `pytest tests/ -x` to validate nothing broke.
- Prefer adding rows to an existing DDT table over writing new one-off test functions.

---

## 3 — Security (OWASP Top 10)

- **Auth** — every new API endpoint must validate the session token. No route may skip auth middleware.
- **Input validation** — validate and sanitize all user-supplied data at the API boundary before it reaches business logic.
- **Injection** — use parameterized queries or the ORM only. String-interpolated SQL is forbidden.
- **Secrets** — no hard-coded credentials, tokens, or keys. Use environment variables or `config.toml` (gitignored). Never commit `config.toml` or `.env`.
- **File uploads** — validate MIME type and magic bytes (not just extension). Store files outside the web root. Enforce size limits.
- **TLS** — all external-facing endpoints must support TLS. See `tests/test_22_tls_combos.py`.
- **Dependencies** — after adding a dependency run `cargo deny check` (Rust) or `pip-audit` (Python). Fix all HIGH/CRITICAL findings before merging.
- **Prompt injection** — sanitize any user text fed to an AI/LLM. Alert the developer if prompt injection is detected.

---

## 4 — Tool Usage

Use the right tool for the job — don't guess or grep manually when a smarter tool exists:

| Need | Tool |
|------|------|
| Understand code / trace execution | `gitnexus_query`, `gitnexus_context` |
| Blast radius before editing | `gitnexus_impact` |
| Safe rename across call graph | `gitnexus_rename` |
| Verify scope of changes before commit | `gitnexus_detect_changes` |
| Exact text / regex search | `grep_search` |
| Find files by name/glob | `file_search` |
| Conceptual / semantic search | `semantic_search` |
| Read source files | `read_file` (large ranges; parallel reads) |
| Run tests / shell commands | `run_in_terminal` |
| Check for compile/lint errors | `get_errors` |
| Multiple file edits at once | `multi_replace_string_in_file` |
| Track multi-step work | `manage_todo_list` |

- Run independent read-only tools in parallel.
- Run write and terminal operations sequentially.
- Always read a file before editing it.

---

## 5 — GitNexus Quick Reference

This repo is indexed as **simple-photos** (11 661 symbols, 22 029 relationships, 300 execution flows).

| Resource | Purpose |
|----------|---------|
| `gitnexus://repo/simple-photos/context` | Overview + index freshness |
| `gitnexus://repo/simple-photos/clusters` | All functional areas |
| `gitnexus://repo/simple-photos/processes` | All execution flows |
| `gitnexus://repo/simple-photos/process/{name}` | Step-by-step trace |

If any GitNexus tool reports a stale index, run `npx gitnexus analyze` first.
