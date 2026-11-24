---
name: test-iterate
description: "Write tests, run them, fix failures, repeat until green — with a hard attempt budget. Use when: implementing a feature, fixing a bug (write the failing test first), a suite is red, adding coverage, doing TDD, or running E2E flows."
argument-hint: "What to test, and the type (unit / integration / E2E)"
---

# Test-Iterate — Red, Green, Refactor

No feature is done without a test that **fails when the feature breaks**. The loop is:
write test → watch it fail for the right reason → fix the code → green.

Note the middle step. A test that has never failed has proven nothing — it may be
asserting something trivially true, and you'd never know.

## When to Use

- Implementing any new feature or function
- Fixing a bug — **write the failing test first**, it's how you prove the fix works
- A red suite
- Verifying a refactor changed nothing observable
- Adding E2E coverage for a user flow

## Test Types

| Type | When |
|---|---|
| Unit | Pure functions, business logic, isolated components |
| Integration | Units working together, DB queries, API handlers |
| E2E | Full user flows — run through the project's own E2E runner in the terminal |

---

## The Budget

**3 fix attempts per failing test.** On the 4th, stop and ask.

The question to answer before attempt 3: *is the test wrong, or is the design wrong?*
A test that is hard to write is usually reporting a real problem — too many dependencies,
hidden global state, a function doing four things. **Fighting a test for three rounds is
a design smell, not a testing problem**, and the fix is often to change the code's shape
rather than the test.

**Never delete a failing test to get green.** If it must be skipped, mark it skipped with
a comment naming the reason and the issue number — a silently deleted test is a lie told
to everyone who reads the suite afterward.

---

## Procedure

### Step 1 — Read the code, then decide what matters

You cannot write a good test for code you haven't read.

```
read_file(filePath, startLine, endLine)
```

Then decide explicitly what's worth testing:

- **The contract** — what does this promise callers?
- **The edges** — empty, null, zero, one, many, max, malformed, wrong type
- **The failure modes** — what should throw, and what should it say?
- **The regression** — for a bug fix, the exact input that broke it

Test behaviour, not implementation. A test asserting internal call order breaks on every
refactor while catching no real bugs.

### Step 2 — Write the test

For new code, test first. For a bug, write the test that reproduces it and **confirm it
fails** before fixing anything.

Conventions:
- Jest/Vitest: `<file>.test.ts` or `__tests__/<file>.test.ts`
- Pytest: `test_<file>.py` or `tests/<file>_test.py`
- Go: `<file>_test.go`, same package

Structure — Arrange, Act, Assert:
```
// Arrange — set up inputs
// Act — call the thing under test
// Assert — verify the observable result
```

Name the test after the behaviour: `returns 400 when email is malformed`, not `test email 2`.
The name is what you read when it fails in CI six months from now.

### Step 3 — Run it, scoped

Run the single test first — a 30-second full suite between attempts destroys the feedback loop.

```powershell
npx vitest run <test-file>
npx jest <test-file> --no-coverage
pytest tests/<file>.py::<test_name> -v
go test ./pkg/... -run TestName
```

### Step 4 — Confirm it fails for the right reason

**This step is the one people skip, and it's the one that matters.**

A new test must fail *because the behaviour is missing*, not because of a typo, a bad
import, or a broken mock. If the failure message isn't the one you expected, fix the test
before touching the implementation.

Then classify honestly:

| The failure means | Do this |
|---|---|
| The implementation is wrong | Fix the code — most common, and the point of the exercise |
| The test is wrong | Fix the test — your understanding of the spec was off |
| The setup/mock is wrong | Fix the test infrastructure |

**Never change an assertion to match wrong output.** That's not fixing, that's writing
down the bug and calling it the spec.

### Step 5 — Fix and re-run

One change, re-run the scoped test. Repeat within budget. If the error is an exception
rather than an assertion failure, hand off to `debug-errors`.

### Step 6 — Full suite

Once the target is green:

```powershell
npm test
```

New failures elsewhere mean your change broke something. **They are your problem** — do
not commit red, and do not assume they were "already failing" without checking.

### Step 7 — E2E through the project's own runner

E2E tests are **committed test files run from the terminal**, not ad-hoc browser driving.
A run you can't repeat in CI proved nothing.

```powershell
npx playwright test                          # whole suite
npx playwright test tests/login.spec.ts      # one spec, while iterating
npx playwright test --headed                 # watch it, when debugging a selector
npx cypress run --spec "cypress/e2e/login.cy.ts"
```

1. Start the dev server via `build-run` (async), or let the runner's `webServer` config
   start it — don't do both, that's an `EADDRINUSE` waiting to happen
2. Run the spec and read the full output
3. On failure, open the runner's own artifacts — Playwright's trace viewer
   (`npx playwright show-trace`) and screenshots-on-failure beat guessing
4. **Kill any dev server you started**

Prefer role- and text-based selectors (`getByRole`, `getByText`) over CSS or XPath — they
survive markup changes, which is where brittle E2E suites actually die.

### Step 8 — Test the guards, not just the happy path

Any code that deletes, overwrites, resets, or migrates needs tests that attack it. A test
proving `deleteExpiredSessions()` deletes expired sessions proves nothing about the case
that matters — **the one where the filter is wrong.**

For every destructive path, assert that the guard *holds*:

```
// the case that actually loses data
it('refuses to delete when the filter is empty', async () => {
  await expect(purge({ filter: {} })).rejects.toThrow(/filter required/)
  expect(await count()).toBe(3)          // ← assert nothing was destroyed
})

it('does not delete on a dry run', async () => {
  await purge({ filter: { expired: true }, dryRun: true })
  expect(await count()).toBe(3)
})

it('refuses a path outside the target root', async () => {
  await expect(removeUpload('../../etc/passwd')).rejects.toThrow(/outside/)
})
```

**Always assert the surviving count**, not just that an error was thrown. A guard can
throw *after* it has already deleted half the rows, and a `rejects.toThrow` alone passes
happily on that.

Also cover: the empty/null filter, the `..` and absolute path, the over-the-cap batch, the
missing `--force` in a non-interactive run, and the migration `down` — run `up` then `down`
and assert the schema and data came back.

**Tests must never touch real state.** A test DB the suite creates and drops, a temp
directory, a fixture — never the dev database, never a path outside the test root, never a
live API. A test suite that can destroy your data is a destructive operation you run on
every commit.

### Step 9 — Coverage, honestly

```powershell
npm test -- --coverage
pytest --cov=src --cov-report=term-missing
```

Target: every new function gets a happy path and at least one edge case. **100% coverage
is a vanity metric** — fully covered code with weak assertions catches nothing. Look at
what's uncovered and ask whether it matters, rather than chasing the number.

## Rules

- **Budget of 3** per failing test, then stop and ask.
- **Watch every new test fail before you make it pass.**
- **Never delete a failing test.** Fix it, or skip it with a documented reason.
- **Don't mock what you can test directly** — over-mocking tests the mock.
- **One concept per test.** A test asserting ten things tells you nothing when it fails.
- **A passing test on broken code means the test is wrong.** Go verify it can fail.
- **Every destructive path gets a test that feeds it the bad input and asserts nothing was
  destroyed** — assert the surviving count, not just that it threw.
- **Tests never touch real state.** Test DB, temp dir, fixtures. Never the dev database.
- Bug fixes require a regression test. No exceptions — that's how the bug stays fixed.
