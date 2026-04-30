# CLO-272 Implement LLMVerifier verify hook with mock-backend fixture

**Status:** draft
**Type:** specification
**Linear:** https://linear.app/cloud-ai/issue/CLO-272/implement-llmverifier-verify-hook-with-mock-backend-fixture
**Design context:** docs/designs/clo-270-hook.md §10-11 (M4 test contract and verify-hook architecture)

## 1. Problem and goal

Implement a concrete `LLMVerifier` verify-hook type that turns a backend model output into a binary pass/fail decision for existing `EscalatingRetry`/future phase-runner flows. This task unlocks FR-15 of the PRD and the LLMVerifier part of CLO-272’s hook pipeline while keeping behavior deterministic and easy to test without network.

Current behavior has `VerifyHook` trait support but no concrete LLM-based verifier, so no end-to-end path exists for using an LLM as a judge for candidate outputs. We need a hook that renders a verification prompt template with `{candidate}`, calls a configured backend, parses a yes/no decision with simple tokenization rules, and returns `VerifyResult::Pass` or `VerifyResult::Fail` with a stable reason. This closes the gap for downstream hook integration while preserving existing failure semantics and API contracts from CLO-270.

The goal is to land a focused, test-first implementation with a `MockBackend`-style fixture test contract, avoiding real HTTP in unit tests and ensuring deterministic verdict parsing for real-world punctuation/casing variations.

## 2. Acceptance criteria

- [ ] **AC1**: `LLMVerifier` type exists in `src/strategy/verify.rs` or a dedicated strategy verify module with fields `backend: BackendId`, `model: Option<String>`, `prompt_template: String`, `system_prompt: Option<String>`, `temperature: f32` (or explicit documented default if omitted). `VerifyHook::name()` for this type returns a stable name (e.g., `LLMVerifier`). (**verification command:** `rg "LLMVerifier" -n src/strategy src | sed -n '1,200p'`)
- [ ] **AC2**: `LLMVerifier` implements `VerifyHook::verify` and renders prompt templates such that `{candidate}` is replaced by `ctx.stdout` while preserving other brace tokens unchanged. (**verification command:** unit test assertions in `tests/verify_llm_verifier.rs` that check substitution behavior)
- [ ] **AC3**: `verify` calls the configured backend through repository-backed backend interfaces with deterministic-as-possible settings, using temperature `0.0` by default when temperature is not explicitly set. (`cargo test --test verify_llm_verifier -- --nocapture candidate_substitution`)
- [ ] **AC4**: Response parsing is deterministic by taking the first whitespace-separated token from the raw backend output, lower-casing, trimming punctuation, and mapping only `yes`→pass and `no`→fail. (`cargo test --test verify_llm_verifier -- --nocapture parse_leading_token_yes_no`)
- [ ] **AC5**: `tests/verify_llm_verifier.rs` contains a passing case for exact response `"yes"` resulting in `VerifyResult::Pass`. (**verification command:** `cargo test --test verify_llm_verifier -- --exact yes_is_pass`)
- [ ] **AC6**: `tests/verify_llm_verifier.rs` contains a passing case for response `"no"` resulting in `VerifyResult::Fail`. (**verification command:** `cargo test --test verify_llm_verifier -- --exact no_is_fail`)
- [ ] **AC7**: `tests/verify_llm_verifier.rs` contains a case where responses like `"Yes."`, `"YES\n"`, and `" yes - because..."` map to pass. (**verification command:** `cargo test --test verify_llm_verifier -- --exact yes_variants_pass`)
- [ ] **AC8**: `tests/verify_llm_verifier.rs` contains a case for ambiguous output (`"maybe"`, empty, whitespace-only) resulting in `VerifyResult::Fail` with reason containing `"unparseable verifier response"`. (**verification command:** `cargo test --test verify_llm_verifier -- --exact unparseable_response_fails`)
- [ ] **AC9**: `tests/verify_llm_verifier.rs` contains a case where backend query errors map to `VerifyResult::Fail` (or `Err(VerifyError)` per current interface) carrying the backend error context without panic/panic unwind. (**verification command:** `cargo test --test verify_llm_verifier -- --exact backend_error_is_fail`)
- [ ] **AC10**: System prompt forwarding is covered by test: when `system_prompt` is set, it is included in backend invocation context/payload in the same run. (**verification command:** `cargo test --test verify_llm_verifier -- --exact forwards_system_prompt`)
- [ ] **AC11**: Template rendering can handle braces other than `{candidate}` by keeping them untouched in prompt output. (**verification command:** `cargo test --test verify_llm_verifier -- --exact non_candidate_braces_passthrough`)
- [ ] **AC12**: Task verification command `cargo test --test verify_llm_verifier` exits successfully after implementation, and no live network calls occur in this test suite (mock/backed fixture only). (**verification command:** `cargo test --test verify_llm_verifier`)

## 3. Sub-tasks

### ST1 Add LLMVerifier verifier struct and hook implementation
**Files:** `src/strategy/verify.rs` (or dedicated `src/strategy/llm_verifier.rs`), `src/strategy/mod.rs`
**Tests:** `tests/verify_llm_verifier.rs`, `src/strategy/verify.rs` module tests
**Estimate:** S

Implement `LLMVerifier` with constructor-like API, implement `VerifyHook`, and wire in deterministic temperature default handling plus template rendering with `{candidate}` substitution. Keep behavior local and explicit (`Fail` on parse ambiguity).

### ST2 Build parse + backend invocation helpers with clear errors
**Files:** `src/strategy/verify.rs` (or dedicated `src/strategy/llm_verifier.rs`)
**Tests:** `tests/verify_llm_verifier.rs`
**Estimate:** S

Add response parser (first token + punctuation trimming/casing + leading token mapping), backend invocation helper that accepts injected backend handle/fixture for testing, and mapping of backend errors into non-panicking failure outcomes.

### ST3 Add dedicated unit test fixture and edge-case coverage
**Files:** `tests/verify_llm_verifier.rs`
**Tests:** all listed AC tests
**Estimate:** M

Create `MockBackend`-style fixture for deterministic backend output and error injection. Cover yes/no parsing permutations, unparseable cases, template substitution semantics, and system prompt forwarding.

### ST4 Hook integration and verification wiring
**Files:** `src/strategy/escalating_retry.rs` (if needed for `VerifyHook` discovery), `src/strategy/verify.rs`
**Tests:** `cargo test --test verify_llm_verifier`, existing strategy integration tests if changed
**Estimate:** S

Ensure `LLMVerifier` is reachable from public strategy exports if currently used externally, and keep interface backward-compatible with CLO-270 `VerifyHook` changes.

### ST5 Pre-merge command gate
**Files:** none (workflow phase state)
**Tests:** `cargo test --test verify_llm_verifier`
**Estimate:** XS

Run unit tests, confirm no network activity in test execution, and update workflow state accordingly.

## 4. Evaluation table

| # | Scenario | Input | Expected | Verification |
|---|---|---|---|---|
| 1 | Candidate is exactly `yes` | `LLMVerifier` over context with `stdout="yes"` | Returns `VerifyResult::Pass` | `cargo test --test verify_llm_verifier -- --exact yes_is_pass` |
| 2 | Candidate is exactly `no` | `LLMVerifier` over context with `stdout="no"` | Returns `VerifyResult::Fail` with reason summary including fail marker | `cargo test --test verify_llm_verifier -- --exact no_is_fail` |
| 3 | Candidate has punctuation variants | `Yes.`, `YES\n`, ` yes - because...` | Parse to leading `yes` and pass | `cargo test --test verify_llm_verifier -- --exact yes_variants_pass` |
| 4 | Candidate is ambiguous | `maybe`, `""` | Returns fail with reason `unparseable verifier response` | `cargo test --test verify_llm_verifier -- --exact unparseable_response_fails` |
| 5 | Backend errors | Mock backend returns error | Error carried into fail path (no panic, no crash) | `cargo test --test verify_llm_verifier -- --exact backend_error_is_fail` |
| 6 | Brace passthrough | Prompt template includes `{"keep": "{this}", "candidate": "{candidate}"}` | Only candidate placeholder substituted | `cargo test --test verify_llm_verifier -- --exact non_candidate_braces_passthrough` |
| 7 | End-to-end task scope | Implemented code and tests | All verify tests pass | `cargo test --test verify_llm_verifier` |

## 5. Edge cases

- **Edge 1:** Backend outputs empty or whitespace-only text — parser treats as unparseable and returns fail with stable reason string so callers can consistently fail closed.
- **Edge 2:** Backend returns `"YES,"` / `"no."` or punctuation-heavy output — tokenizer splits on whitespace and strips punctuation before token match to avoid false negatives.
- **Edge 3:** Backend throws transient or non-transient errors — hook returns fail-like outcome with backend error context (or equivalent verify error mapped in tests) and does not panic/unwrap.
- **Edge 4:** Template contains brace-like text for external format syntax — all braces are treated as raw text except `{candidate}` replacement.
- **Edge 5:** Deterministic settings unavailable in some backends — implementation still chooses stable default temperature path and documents capability fallbacks.
