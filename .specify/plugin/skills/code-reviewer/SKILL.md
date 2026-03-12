---
name: code-reviewer
description: AI-powered code review for generated Rust crates, catching security issues and quality problems
argument-hint: [crate-path] [--fix?]
allowed-tools: Read, Write, StrReplace, Shell, Grep
---

# AI Code Review Skill

## Overview

Perform comprehensive AI-powered code review on generated Rust WASM crates, identifying security vulnerabilities, missing validation, performance issues, and code quality problems.

**Research Validation**: Studies show AI-generated code has **1.7× more issues than human code**, with specific weaknesses in:

- Missing null checks and error handling
- Security vulnerabilities (SQL injection, XSS, command injection)
- Excessive I/O operations (~8× more common)
- Unclear naming and poor readability

**AI-on-AI Review**: Using AI to review AI-generated code catches issues one model missed—a paradoxical but effective quality gate.

### Why Code Review?

#### Common Issues in AI-Generated Code

Research findings on AI code quality:

1. **10.83 issues per PR** vs 6.45 for human code (1.68× multiplier)
2. **Security vulnerabilities** appear 1.5-2× more frequently
3. **Missing guardrails**: No null checks, missing early returns, inadequate exception logic
4. **Excessive I/O**: ~8× more likely to have inefficient I/O patterns
5. **Unclear naming**: Generic identifiers increase cognitive load
6. **Business logic errors**: Incorrect dependencies, flawed control flow

#### This Skill's Value

- **Automated detection** of common AI code issues
- **Specific fixes** for critical problems (not just "check this")
- **Auto-fix capability** for simple issues (--fix flag)
- **Educational feedback** to improve future generations

## Derived Arguments

1. **Crate Path** (`$CRATE_PATH`): Path to generated Rust crate
2. **Auto-fix Flag** (`$AUTO_FIX`): Optional `--fix` flag to automatically repair critical issues

```text
$CRATE_PATH = $ARGUMENTS[0]
$AUTO_FIX   = "--fix" in $ARGUMENTS  # Boolean
$REVIEW_OUTPUT = $CRATE_PATH/REVIEW.md
```

## Prerequisites

- Generated Rust crate (from `crate-writer`)
- Crate must compile (`cargo check` passes)

## Review Categories

### 1. Security (CRITICAL)

Issues that could lead to data breaches, unauthorized access, or system compromise.

**Check for**:

- SQL injection vulnerabilities
- Command injection (shell execution with user input)
- XSS in HTML/XML output
- Path traversal vulnerabilities
- Hardcoded secrets or credentials
- Unsafe deserialization
- Missing authentication checks

**Severity**: CRITICAL (must fix before deployment)

### 2. Error Handling (CRITICAL)

Missing error handling leads to panics and service outages.

**Check for**:

- `unwrap()` or `expect()` calls in production code
- Unhandled `Option::None` cases
- Unhandled `Result::Err` cases
- Errors that aren't propagated with `?`
- Generic error messages (no context)
- Swallowed errors (caught but not logged)

**Severity**: CRITICAL (causes runtime panics)

### 3. WASM Constraints (CRITICAL)

Violations prevent compilation or cause runtime errors in WASM.

**Check for**:

- `std::env` usage (must use Config provider)
- `std::fs` usage (must use StateStore provider)
- `std::net` usage (must use HttpRequest provider)
- `std::thread` usage (must be async)
- Mutable global state (`static mut`, `OnceCell` outside `LazyLock` pattern)
- `unsafe` code blocks
- Blocking operations (synchronous I/O)

**Severity**: CRITICAL (build failure or runtime crash)

### 4. Provider Misuse (HIGH)

Incorrect use of Omnia SDK providers.

**Check for**:

- Missing provider trait bounds on handlers
- Direct system calls instead of providers
- Provider methods called incorrectly
- Missing error handling on provider calls

**Severity**: HIGH (functional bugs)

### 5. Validation Logic (HIGH)

Missing or misplaced validation causes incorrect behavior.

**Check for**:

- No validation on required fields
- Structural validation in `handle()` instead of `from_input()`
- Temporal validation in `from_input()` instead of `handle()`
- Missing format validation (email, URL, phone)
- Missing range checks (amount > 0, length <= 1000)
- No business rule validation

**Severity**: HIGH (accepts invalid data)

### 6. Performance (MEDIUM)

Inefficient patterns that cause slow response times.

**Check for**:

- N+1 query patterns (loop with API calls)
- Excessive HTTP requests (not batched)
- Missing caching for repeated data
- Large allocations in hot paths
- Unnecessary cloning
- Synchronous operations in async context

**Severity**: MEDIUM (performance degradation)

### 7. Code Quality (LOW)

Readability and maintainability issues.

**Check for**:

- Unclear variable names (`data`, `tmp`, `x`, `result`)
- Functions > 50 lines (consider splitting)
- Missing documentation for complex logic
- Inconsistent naming (snake_case violations)
- Dead code or unused variables
- Magic numbers (should be named constants)

**Severity**: LOW (technical debt)

## Process

This skill uses an agent team with 3 specialist reviewers and 1 antagonist. The lead coordinates the team, synthesizes findings, and produces the final `REVIEW.md`. See [Agent Team Patterns](references/agent-teams.md) for shared protocols.

### Step 1: Initialize Team

**CREATE** agent team with 4 teammates. Each receives the crate path and their assigned review categories.

**Spawn Security Reviewer**:

```text
You are a Security Reviewer for a Rust WASM crate at $CRATE_PATH.

Your assigned categories: Security and WASM Constraints.

SECURITY: Scan every .rs file in src/ for:
- SQL injection (string concatenation in queries)
- Command injection (shell execution with user input)
- XSS (unescaped user input in HTML/XML output)
- Path traversal vulnerabilities
- Hardcoded secrets or credentials (API keys, passwords, tokens in source)
- Unsafe deserialization
- Missing authentication checks

WASM CONSTRAINTS: Scan every .rs file in src/ for:
- std::env usage (must use Config provider)
- std::fs usage (must use StateStore provider)
- std::net usage (must use HttpRequest provider)
- std::thread usage (must be async)
- Mutable global state (static mut, OnceCell outside LazyLock)
- unsafe code blocks
- Blocking operations (synchronous I/O)

For each finding, report: file:line, code snippet, severity (CRITICAL for all
in these categories), risk description, suggested fix, and whether it is
auto-fixable.

Output your findings as a numbered list in markdown. Prefix each finding ID
with "SEC-" (e.g., SEC-1, SEC-2).
```

**Spawn Correctness Reviewer**:

```text
You are a Correctness Reviewer for a Rust WASM crate at $CRATE_PATH.

Your assigned categories: Error Handling, Validation Logic, and Provider Misuse.

ERROR HANDLING: Search all .rs files in src/ for:
- unwrap() or expect() calls in production code (not tests)
- Unhandled Option::None or Result::Err cases
- Errors not propagated with ?
- Generic error messages without context
- Swallowed errors (caught but not logged or returned)

VALIDATION LOGIC: Read all from_input() and handle() methods:
- Structural validation (required fields, format, range) must be in from_input()
- Temporal validation (Utc::now(), runtime state) must be in handle()
- Missing validation on required fields or user input
- Missing format validation (email, URL, phone)

PROVIDER MISUSE: Check handler functions for:
- Missing provider trait bounds
- Direct system calls instead of provider methods
- Provider methods called incorrectly
- Missing error handling on provider calls

For each finding, report: file:line, code snippet, severity (CRITICAL for
error handling panics, HIGH for validation/provider issues), risk, suggested
fix, auto-fixable status.

Output your findings as a numbered list in markdown. Prefix each finding ID
with "COR-" (e.g., COR-1, COR-2).
```

**Spawn Quality Reviewer**:

```text
You are a Quality Reviewer for a Rust WASM crate at $CRATE_PATH.

Your assigned categories: Performance and Code Quality.

PERFORMANCE: Scan all .rs files in src/ for:
- N+1 query patterns (HTTP/DB calls inside loops)
- Excessive HTTP requests (not batched)
- Missing caching for repeated data lookups
- Large allocations in hot paths
- Unnecessary cloning (.clone() where a reference suffices)
- Synchronous operations in async context

CODE QUALITY: Check all .rs files in src/ for:
- Unclear variable names (data, tmp, x, result, value)
- Functions longer than 50 lines
- Missing documentation on complex logic
- Inconsistent naming (snake_case violations)
- Dead code or unused variables
- Magic numbers (should be named constants)

For each finding, report: file:line, code snippet, severity (MEDIUM for
performance, LOW for code quality), impact description, suggested fix,
auto-fixable status.

Output your findings as a numbered list in markdown. Prefix each finding ID
with "QUA-" (e.g., QUA-1, QUA-2).
```

**Spawn Antagonist** (after specialists complete):

```text
You are the Antagonist Reviewer for a Rust WASM crate at $CRATE_PATH.

You receive findings from three specialist reviewers (Security, Correctness,
Quality). Your job is to challenge every finding and find what they missed.

For EACH specialist finding:
1. Validate evidence: Is there a real file:line reference and code snippet?
2. Challenge severity: Is CRITICAL really critical? Is LOW actually higher?
3. Check for false positives: Could this be a non-issue or acceptable pattern?
4. Assess auto-fix safety: Could the suggested fix introduce regressions?

Then perform a COUNTER-SCAN of all .rs files in src/ looking for issues ALL
THREE specialists missed. Common blind spots:
- Error handling in edge paths (not just main handlers)
- Subtle type confusion (newtypes used inconsistently)
- Race conditions in async code
- Missing error context chains (? without .context())
- Serde attribute mistakes (rename vs rename(deserialize))

Output format:
## Confirmed: [ID] -- evidence solid, severity accurate
## Downgraded: [ID] ORIG_SEVERITY -> NEW_SEVERITY -- rationale
## Upgraded: [ID] ORIG_SEVERITY -> NEW_SEVERITY -- rationale
## Disputed: [ID] -- rationale (must cite evidence for dispute)
## New Findings: NEW-1, NEW-2, etc. with full finding details

You MUST provide evidence for every challenge. Opinion alone is insufficient.
You CANNOT remove findings entirely -- minimum action is downgrade to LOW.
Severity downgrades move at most one level (CRITICAL to HIGH, not to LOW).
```

### Step 2: Specialist Analysis (Concurrent)

The three specialists analyze the crate concurrently. Each reads all `.rs` files in `src/` but reports only on their assigned categories.

**Lead waits** for all three specialists to complete before proceeding.

### Step 3: Adversarial Challenge

After all specialists report, the lead sends their combined findings to the antagonist.

The antagonist:

1. Reviews every specialist finding for evidence quality and severity accuracy
2. Performs a counter-scan for missed issues
3. Sends challenged report to lead with: confirmed, downgraded, upgraded, disputed, and new findings

### Step 4: Synthesis

The lead merges all findings into `$CRATE_PATH/REVIEW.md`:

1. **Confirmed findings**: Include verbatim from specialist reports
2. **Downgraded findings**: Include with the antagonist's revised severity and rationale
3. **Upgraded findings**: Include with the antagonist's revised severity and rationale
4. **Disputed findings**: Lead makes final call; if included, add dispute note
5. **New findings**: Include with the antagonist's severity and evidence
6. Assign overall confidence level per [Agent Team Patterns - Confidence Scoring](references/agent-teams.md#confidence-scoring)
7. Add "Adversarial Review" section documenting challenge statistics

### Step 5: Auto-Fix (if --fix flag provided)

If `$AUTO_FIX == true`:

The **lead** applies all auto-fixes directly (specialists and antagonist have completed their analysis at this point). The finding prefix (SEC-, COR-, QUA-) tracks which reviewer identified the issue for accountability in the report.

**FOR EACH** confirmed or upgraded auto-fixable issue (not disputed):

1. **Verify** fix is safe (no side effects, antagonist did not flag regression risk)
2. **Apply** fix using Edit tool
3. **Mark** issue as "Fixed" in report, noting the originating reviewer prefix
4. **Add** to auto-fix log

**RE-CHECK**: Run `cargo check` to verify fixes compile

```bash
cd $CRATE_PATH && cargo check 2>&1
```

If errors introduced:

- **REVERT** all auto-fixes
- **WARN** in report: "Auto-fix caused compilation errors; manual review required"

### Step 6: Cleanup

Lead shuts down all teammates and cleans up the agent team.

### Review Report Template

````markdown
# Code Review Report

**Generated**: [timestamp]
**Crate**: [name]
**Review Team**: 3 specialists + 1 antagonist
**Auto-fix**: [enabled/disabled]
**Confidence Level**: [HIGH | MEDIUM | LOW]

---

## Summary

- 🔴 Critical Issues: [count]
- 🟠 High Severity: [count]
- 🟡 Medium Severity: [count]
- 🔵 Low Severity: [count]

**Overall Assessment**: [Excellent | Good | Fair | Poor]

---

## 🔴 Critical Issues (MUST FIX)

### SEC-1: WASM Constraint Violation

**File**: [src/config.rs:23](src/config.rs#L23)
**Category**: WASM Compliance
**Reviewer**: Security Reviewer
**Antagonist**: ✅ Confirmed

**Issue**: Direct environment variable access (std::env)

\```rust
let api_url = std::env::var("API_URL").unwrap();
\```

**Risk**: Compilation failure or runtime panic in WASM
**Fix Applied**: ✅ Auto-fixed

\```rust
let api_url = ctx.config.get("API_URL")?;
\```

---

### COR-1: Missing Error Handling (Potential Panic)

**File**: [src/handlers.rs:67](src/handlers.rs#L67)
**Category**: Error Handling
**Reviewer**: Correctness Reviewer
**Antagonist**: ⬆️ Upgraded from HIGH to CRITICAL (untrusted input path)

[... finding details ...]

---

## 🟠 High Severity Issues

[... high severity findings ...]

## 🟡 Medium Severity Issues

[... medium severity findings ...]

## 🔵 Low Severity Issues

[... low severity findings ...]

---

## Adversarial Review

**Antagonist Activity Summary**:

| Action       | Count   |
| ------------ | ------- |
| Confirmed    | [count] |
| Downgraded   | [count] |
| Upgraded     | [count] |
| Disputed     | [count] |
| New Findings | [count] |

**Acceptance Rate**: [confirmed / total specialist findings]%

### Downgraded Findings

- [COR-3] HIGH → MEDIUM: Missing length validation on description field
  **Rationale**: Field is bounded by serde max_length attribute at deserialization

### Upgraded Findings

- [COR-1] HIGH → CRITICAL: unwrap() on untrusted input path
  **Rationale**: Input comes directly from HTTP request body; attacker-controlled

### Disputed Findings

- [SEC-5] Reported as CRITICAL: "potential SQL injection"
  **Dispute**: No SQL database used; query passed to HttpRequest provider
  **Lead Decision**: Excluded (antagonist rationale accepted)

### New Findings (Missed by Specialists)

- [NEW-1] CRITICAL: Missing error propagation in retry loop (src/handlers.rs:112)
  **Evidence**: errors.push(e) swallows errors; Ok(()) returned when all retries fail

---

## Auto-Fix Summary

**Total Fixes Applied**: [count]
**Successful**: [count]
**Failed**: [count]

**Modified Files**:

- src/handlers.rs ([count] fixes)
- src/config.rs ([count] fixes)
- src/types.rs ([count] fixes)

**Verification**: [✅ cargo check passed | ⚠️ reverted due to errors]

---

## Quality Metrics

| Metric                   | This Crate | AI Baseline | Human Baseline | Status   |
| ------------------------ | ---------- | ----------- | -------------- | -------- |
| Issues per 100 LOC       | [n]        | 1.8         | 1.1            | [status] |
| Critical issues          | [n]        | 5 (est.)    | 2 (est.)       | [status] |
| Missing error handling   | [n]        | 15 (est.)   | 5 (est.)       | [status] |
| Security vulnerabilities | [n]        | 2 (est.)    | 0              | [status] |

---

## Next Steps

1. [✅ | ⏭️] Auto-fixes applied and verified
2. ⏭️ Manual review of remaining critical issues
3. ⏭️ Address antagonist new findings
4. ⏭️ Integration testing
5. ⏭️ Security audit (cargo-audit)

---

**End of Review Report**
````

## Reference Documentation

- [Agent Team Patterns](references/agent-teams.md) -- Shared team roles, antagonist protocol, synthesis rules, and file ownership
- [CodeRabbit Study: AI Code Creates 1.7x More Issues](https://www.coderabbit.ai/blog/state-of-ai-vs-human-code-generation-report)
- [Security Best Practices for Rust](https://anssi-fr.github.io/rust-guide/)
- [WASM Security Model](https://webassembly.org/docs/security/)

## Examples

### Example 1: Simple CRUD Review

**Input**: `crate-path` pointing to a generated order management crate

**Review finds**:

- Critical: `unwrap()` on user-provided customer ID lookup
- High: Missing input length validation on `description` field
- Medium: N+1 HTTP calls in order listing endpoint

**Auto-fix applies**: Replaces `unwrap()` with explicit Omnia-compatible errors (for example `bad_request!` or `Error::NotFound { code, description }`). Remaining issues documented in REVIEW.md.

### Example 2: Complex Workflow Review

**Input**: `crate-path` pointing to a payment processing crate

**Review finds**:

- Critical: Hardcoded API key in test fixture (should use Config provider)
- Critical: Missing error propagation on HTTP timeout
- High: `std::thread::sleep` used instead of `tokio::time::sleep`

**Auto-fix applies**: Replaces `std::thread::sleep` with async equivalent, adds `?` operator for error propagation. Hardcoded key flagged for manual fix.

## Error Handling

### Common Issues and Resolutions

| Issue                              | Cause                                       | Resolution                                                               |
| ---------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------ |
| Crate path not found               | Incorrect `$CRATE_PATH` argument            | Verify the path exists and contains `src/` with `.rs` files              |
| No `.rs` files in `src/`           | Crate not yet generated or wrong directory  | Run `crate-writer` first, then re-run code review                           |
| `cargo check` fails after auto-fix | Auto-fix introduced a compilation error     | Revert all auto-fixes and document issues for manual resolution          |
| Review report empty                | All files excluded or no issues found       | Verify `src/` directory is not empty; check file permissions             |
| Auto-fix modifies test files       | Test code scanned alongside production code | Review should focus on `src/` only; exclude `tests/` from auto-fix scope |

### Recovery Process

1. If auto-fix caused compilation errors: revert changes with `git checkout -- src/`
2. Re-run review with `--no-fix` to get a report without auto-fixes
3. Apply fixes manually based on the report recommendations
4. Run `cargo check` and `cargo test` after each manual fix
5. Re-run review to verify issues are resolved

## Verification Checklist

Before completing review:

### Team Execution

- [ ] All 3 specialists spawned with correct category assignments
- [ ] All specialists completed before antagonist spawned
- [ ] Antagonist received all specialist findings
- [ ] Antagonist provided evidence for every challenge
- [ ] Lead synthesized all findings into REVIEW.md
- [ ] Team shut down and cleaned up

### Scan Coverage

- [ ] Security Reviewer: SQL injection, XSS, secrets, WASM constraints checked
- [ ] Correctness Reviewer: unwrap/expect, validation placement, provider usage checked
- [ ] Quality Reviewer: N+1 patterns, naming, function length, dead code checked
- [ ] Antagonist: counter-scan completed for blind spots

### Report Quality

- [ ] Each issue has file:line reference and code snippet
- [ ] Severity reflects antagonist adjustments (upgrades/downgrades applied)
- [ ] Adversarial Review section included with challenge statistics
- [ ] Confidence level assigned based on antagonist results
- [ ] Finding IDs use correct prefixes (SEC-, COR-, QUA-, NEW-)

### Auto-Fix (if enabled)

- [ ] Only confirmed or upgraded auto-fixable issues fixed (not disputed)
- [ ] Antagonist regression flags respected (no fix applied if flagged)
- [ ] Lead applied all fixes (not delegated to specialists)
- [ ] All fixes verified with `cargo check`
- [ ] Modified files listed with originating reviewer prefix (SEC-, COR-, QUA-)
- [ ] Revert performed if errors introduced

## Important Notes

### Expected Results

#### Typical Issue Counts by Crate Complexity

**Simple CRUD** (200-300 LOC):

- Critical: 0-2
- High: 2-5
- Medium: 1-3
- Low: 5-10

**Business Logic** (500-800 LOC):

- Critical: 2-5
- High: 5-10
- Medium: 3-6
- Low: 10-20

**Complex Workflows** (1000+ LOC):

- Critical: 5-10
- High: 10-20
- Medium: 5-10
- Low: 20-40

#### Auto-Fix Success Rate

- **Error handling (unwrap→?)**: 90% success
- **WASM violations (std::env→Config)**: 80% success
- **Missing validation**: 60% success (some require business logic understanding)
- **Performance issues**: 10% success (most need architectural changes)
- **Code quality**: 5% success (semantic understanding required)

**Overall auto-fix rate**: ~40-50% of issues

### Integration with `/spec:apply`

Add code review near the end of implementation, after generation and verification:

```bash
/code-reviewer $CRATE_PATH --fix

if grep "Critical Issues: [1-9]" $CRATE_PATH/REVIEW.md; then
    echo "Critical issues found - manual review required"
    echo "See $CRATE_PATH/REVIEW.md for details"
fi
```
