# Agent Team Patterns

Shared patterns for skills that coordinate work through Agent Teams. Each skill references this document and provides skill-specific values for the parameterized sections below.

## Team Roles

Every agent team has exactly three role types. Skills define which specialists to spawn and what each one owns.

| Role | Count | Responsibility |
| ---- | ----- | -------------- |
| **Lead** | 1 (the invoking session) | Creates team, spawns teammates, assigns tasks, approves plans, synthesizes final deliverable |
| **Specialist** | 1–5 | Performs focused work within an assigned scope; reports findings or proposes plans to the lead |
| **Antagonist** | 1 | Challenges all specialist outputs; identifies false positives, missed issues, and severity misratings |

### Lead Responsibilities

1. Spawn teammates with detailed spawn prompts (scope, file ownership, output format)
2. Monitor shared task list; reassign work if a teammate stalls
3. Wait for all specialists to complete before spawning the antagonist
4. Review antagonist challenges and make final decisions
5. Synthesize all teammate outputs into a single deliverable
6. Shut down teammates and clean up the team when done

### Specialist Responsibilities

1. Work only within assigned scope (categories, files, hypotheses)
2. Produce structured output in the format the lead specified
3. Provide evidence for every claim (file:line references, code snippets, error messages)
4. Do not modify files outside assigned ownership (see File Ownership Rules)
5. Send findings to the lead when complete

### Antagonist Responsibilities

1. Wait for all specialists to finish before starting
2. Read every specialist's output
3. For each finding or proposal, evaluate:
   - **Evidence quality**: Is there a concrete file:line reference and code snippet?
   - **Severity accuracy**: Does the assigned severity match the actual risk?
   - **False positive risk**: Could this be a non-issue or acceptable pattern?
   - **Regression risk** (for fix proposals): Will this fix break something else?
4. Perform a counter-scan for issues or approaches all specialists missed
5. Send challenged report to lead with categorized results

## Antagonist Protocol

The antagonist is the quality gate. Its output determines the final confidence level of the deliverable.

### Challenge Categories

For each specialist finding or proposal, the antagonist assigns one of:

| Category | Meaning | Action |
| -------- | ------- | ------ |
| **Confirmed** | Evidence is solid, severity is accurate | Include as-is in final deliverable |
| **Downgraded** | Finding is real but severity is too high | Include with reduced severity + rationale |
| **Upgraded** | Finding is real but severity is too low | Include with increased severity + rationale |
| **Disputed** | Insufficient evidence or likely false positive | Include at LOW with dispute rationale; lead makes final call |
| **New Finding** | Issue missed by all specialists | Include with severity + evidence |

### Antagonist Rules

1. **Evidence required**: Every challenge must cite specific code, test output, or documentation. Opinion alone is insufficient.
2. **No removals**: The antagonist cannot remove findings entirely. The minimum action is downgrade to LOW with strong justification.
3. **Severity bounds**: Downgrades move at most one level (CRITICAL to HIGH, not CRITICAL to LOW). Upgrades have no bound.
4. **Root cause focus** (for fix proposals): Challenge fixes that address symptoms rather than root causes. A fix that silences a test without addressing the underlying issue must be flagged.
5. **Regression awareness**: For every proposed code change, the antagonist must assess which other tests or behaviors could break.

### Antagonist Output Format

```markdown
## Antagonist Review

### Confirmed Findings
- [S1-F3] CRITICAL: unwrap() on user input (src/handlers.rs:67) -- evidence solid, severity accurate

### Downgraded Findings
- [S2-F1] HIGH → MEDIUM: Missing length validation on description field
  Rationale: Field is bounded by serde max_length attribute at deserialization; risk is lower than stated

### Upgraded Findings
- [S3-F2] LOW → HIGH: Generic variable name `data` in hot path (src/handlers.rs:89)
  Rationale: This shadows an outer `data` binding, causing a logic bug, not just readability

### Disputed Findings
- [S1-F5] Reported as CRITICAL: "potential SQL injection"
  Dispute: No SQL database is used; query string is passed to HttpRequest provider which handles escaping

### New Findings
- CRITICAL: Missing error propagation in retry loop (src/handlers.rs:112-118)
  Evidence: `errors.push(e)` swallows errors silently; final `Ok(())` returned even when all retries fail
```

## Synthesis Protocol

After the antagonist completes, the lead produces the final deliverable.

### Synthesis Rules

1. **Confirmed findings**: Include verbatim from specialist reports
2. **Downgraded findings**: Include with the antagonist's revised severity and rationale
3. **Upgraded findings**: Include with the antagonist's revised severity and rationale
4. **Disputed findings**: Lead makes final call; if included, add dispute note; if excluded, log in "Excluded Disputes" section
5. **New findings**: Include with the antagonist's severity and evidence
6. **Adversarial Review section**: Add a summary of antagonist activity (challenges made, acceptance rate, new findings) to the final deliverable

### Confidence Scoring

The lead assigns a confidence level based on antagonist results:

| Condition | Confidence |
| --------- | ---------- |
| Antagonist confirmed > 80% of findings, no new CRITICAL findings | HIGH |
| Antagonist challenged 20-40% of findings, or added MEDIUM+ findings | MEDIUM |
| Antagonist challenged > 40% of findings, or added CRITICAL findings | LOW |

## File Ownership Rules

Teammates must not edit files outside their assigned ownership. This prevents merge conflicts and ensures accountability.

### Assignment Principles

1. **No overlapping ownership**: Two teammates must never own the same file
2. **Read access is unrestricted**: Any teammate can read any file for analysis
3. **Ownership follows scope**: A teammate that reviews `types.rs` should own fixes to `types.rs`
4. **Shared files go to lead**: Files that span multiple scopes (e.g., `Cargo.toml`, `lib.rs`) are owned by the lead
5. **Tests are last-resort edits**: Test file modifications require explicit lead approval and should only happen when the artifacts are clearly wrong

### Common Ownership Patterns

**Code Review Teams** (read-only analysis; lead applies auto-fixes if `--fix`):

| Teammate | Role |
| -------- | ---- |
| Security Reviewer | Read-only analysis; findings prefixed SEC- |
| Correctness Reviewer | Read-only analysis; findings prefixed COR- |
| Quality Reviewer | Read-only analysis; findings prefixed QUA- |
| Antagonist | Read-only challenges; no file ownership |
| Lead | Applies all auto-fixes; owns `REVIEW.md` and all `src/` files during fix phase |

**Repair Hypothesis Teams** (plan approval workflow):

| Teammate | Owned Files (if plan approved) |
| -------- | ------------------------------ |
| Type/Structural | `src/types.rs`, model modules |
| Logic/Validation | `src/handlers.rs`, domain logic modules |
| Test/Artifact Alignment | `tests/*.rs` (lead approval required) |
| Antagonist | No file ownership (challenges only) |
| Lead | `Cargo.toml`, `src/lib.rs`, cross-cutting files |

**Batch Pipeline Teams**:

| Teammate | Owned Files |
| -------- | ----------- |
| Project teammate N | `$CRATE_DIR/<crate_name>/`, `$WORK_DIR/<crate_name>*` |
| Lead | Batch summary, checkpoint file, shared project root files |

## Plan Approval Workflow

Used when teammates propose changes rather than implementing directly (e.g., TDD-Gen repair hypotheses).

### Flow

1. Lead spawns teammates with `require plan approval` in spawn prompt
2. Each teammate analyzes the problem and produces a fix plan (read-only)
3. Antagonist reviews all plans and sends ranked assessment to lead
4. Lead evaluates plans against approval criteria and antagonist ranking
5. Lead approves the best plan (or combines elements from multiple plans)
6. Approved teammate exits plan mode and implements the fix
7. Lead verifies the fix (e.g., runs tests)
8. If fix fails, lead starts a new iteration with updated context

### Plan Approval Criteria

The lead approves plans that satisfy all of:

1. **Root cause**: Addresses the underlying cause, not just the symptom
2. **Minimal scope**: Changes the fewest files and lines necessary
3. **Low regression risk**: Antagonist assessment shows low chance of breaking other tests
4. **Pattern compliance**: Fix follows established patterns in [repair-patterns.md](../plugins/omnia/skills/crate-writer/references/repair-patterns.md)
5. **No test modification** (unless artifacts are demonstrably wrong): Fixes the implementation, not the tests

### Cross-Iteration Memory

When a team is re-spawned for a subsequent iteration, the lead includes in each teammate's spawn prompt:

- Previous iteration's test output
- Which hypotheses were tried and their outcomes
- Which files were modified and how
- Explicit instruction to avoid previously failed approaches

## Team Sizing Guidelines

| Scenario | Team Size | Rationale |
| -------- | --------- | --------- |
| Batch processing (N projects) | min(N, 5) specialists | One teammate per project; cap at 5 to manage token cost |
| Code review | 3 specialists + 1 antagonist | One per review domain; antagonist challenges all |
| TDD-Gen repair | 3 specialists + 1 antagonist | One per hypothesis category; antagonist ranks plans |

Exceeding 5 teammates increases coordination overhead and token cost without proportional quality improvement. When batch items exceed 5, queue excess items and assign to teammates as they complete their current task.
