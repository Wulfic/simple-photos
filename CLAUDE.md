<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **simple-photos** (11661 symbols, 22029 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/simple-photos/context` | Codebase overview, check index freshness |
| `gitnexus://repo/simple-photos/clusters` | All functional areas |
| `gitnexus://repo/simple-photos/processes` | All execution flows |
| `gitnexus://repo/simple-photos/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->

---

# Project Development Standards

## 1 — Always Plan First

Before writing any code or running any command that changes state:

1. **Write a plan** — list every file, function, and migration that will be touched.
2. **Check impact** — run `gitnexus_impact` on every symbol in the plan.
3. **Share the plan** with the user and get explicit approval before proceeding.
4. **Track progress** with the `manage_todo_list` tool: one item in-progress at a time, mark completed immediately.

Never begin implementation without a written, approved plan.

## 2 — Testing: DDT and E2E First

This project uses a numbered test suite (`tests/test_NN_*.py`). Follow these rules whenever building, fixing, or refactoring features:

### Data-Driven Tests (DDT/parametrized)
- Every code path that accepts variable input **must** be covered by a `pytest.mark.parametrize` table.
- Follow the pattern in `tests/test_35_edit_save_ddt.py` and `tests/test_38_edit_dimensions_ddt.py`:
  - One parameter table per logical group (`BRIGHTNESS_CASES`, `ROTATION_CASES`, etc.).
  - Each row is a `pytest.param(...)` with a descriptive `id=` string.
  - Test logic is shared; only the inputs change per row.
- Boundary values, minimums, maximums, and invalid inputs must each be a distinct row.

### End-to-End (E2E) Tests
- Every user-facing feature **must** have at least one E2E test that exercises the full request → server → storage → response cycle via `APIClient` (see `tests/helpers.py`).
- New test files follow the next sequential number: check the highest existing `test_NN_` prefix and increment by one.
- E2E tests **must** run against the real server (`conftest.py` fixture), never mocked internals.
- After implementing a feature, run the relevant test file(s) with `pytest tests/test_NN_*.py -v` and confirm all pass before considering the work done.

### General testing rules
- Do not remove or weaken existing assertions.
- `helpers.py` utilities (`APIClient`, `generate_test_jpeg`, etc.) are shared — changes there affect every test; run `pytest tests/ -x` after any edit to `helpers.py`.
- Prefer adding rows to an existing DDT table over creating new one-off test functions.

## 3 — Security

Follow OWASP Top 10 at all times. Specific rules for this codebase:

- **Authentication & Authorization** — every new API endpoint must validate the session token. Never add a route that skips the auth middleware.
- **Input validation** — validate and sanitize all user-supplied data at the system boundary (API layer). Reject unexpected types, lengths, and characters before they reach business logic.
- **SQL / query injection** — use parameterized queries or the ORM only. String-interpolated queries are forbidden.
- **Secrets** — never hard-code credentials, tokens, or keys. Read them from environment variables or `config.toml` (which is gitignored). Never commit `config.toml` or any `.env` file.
- **File uploads** — validate MIME type and magic bytes, not just the file extension. Store uploaded files outside the web root. Enforce size limits.
- **TLS** — all external-facing endpoints must support TLS (see `tests/test_22_tls_combos.py` for the test pattern).
- **Dependency audit** — after adding any new dependency run `cargo deny check` (Rust) or `pip-audit` (Python) and fix all HIGH/CRITICAL findings before merging.
- **Prompt injection** — if any code processes user-supplied text that is later fed to an AI/LLM, sanitize it. Alert the developer if prompt injection is detected.

## 4 — Tool Usage

Use the full set of available tools rather than guessing or grepping manually:

| Need | Tool |
|------|------|
| Understand code / architecture | `gitnexus_query`, `gitnexus_context` |
| Blast radius of a change | `gitnexus_impact` |
| Safe rename across call graph | `gitnexus_rename` |
| Detect unexpected changes before commit | `gitnexus_detect_changes` |
| Exact text search in files | `grep_search` |
| Find files by name/glob | `file_search` |
| Conceptual / semantic search | `semantic_search` |
| Read a file | `read_file` (prefer large ranges over many small reads) |
| Run tests / shell commands | `run_in_terminal` |
| Validate no new compile errors | `get_errors` |
| Multi-file edits | `multi_replace_string_in_file` |

Prefer read-only tools in parallel; run write/terminal operations sequentially. Always read a file before editing it.
