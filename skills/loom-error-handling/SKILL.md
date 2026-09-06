---
name: loom-error-handling
description: Error handling patterns and strategies including Rust Result/Option, API error responses, data pipeline errors, and security-aware handling.
triggers:
  - exception
  - try
  - catch
  - throw
  - raise
  - Result
  - Option
  - panic
  - recover
  - retry
  - fallback
  - graceful degradation
  - circuit breaker
  - error boundary
  - "500"
  - 4xx
  - 5xx
  - thiserror
  - anyhow
  - RFC 7807
  - RFC 9457
  - problem+json
  - error propagation
  - error context
  - error messages
  - stack trace
---

# Error Handling

## Overview

Cross-language error-handling reference. The recurring mistakes are not syntactic: swallowing errors, retrying non-idempotent operations, losing the cause chain when wrapping, leaking internals in API responses, and log-and-rethrow double logging. This skill leads with the decision rules an expert applies, then shows the minimum code to implement each.

## Agent Delegation

- **loom-senior-software-engineer** (Opus) — DEFAULT. Error architecture, strategy choice, secure handling (no info leak), infra resilience (retry/circuit breaker/fallback).
- **loom-software-engineer** (Sonnet) — ONLY boilerplate error types / unit tests following an established pattern.

## Error Taxonomy — Classify First

Every failure is exactly one of three kinds; the kind dictates handling:

| Kind | Meaning | Handle by |
| --- | --- | --- |
| **Expected / recoverable** | Invalid input, not-found, transient network/rate-limit | Return a typed error (`Result`/checked exception); retry only if transient AND idempotent |
| **Bug / programmer error** | Broken invariant, unreachable state, index OOB | Fail loud: `panic!`/assert/throw. Do NOT try to recover — it hides the bug |
| **Fatal / environmental** | OOM, disk full, config missing at startup | Fail fast at the boundary; crash cleanly, let the supervisor restart |

⚠ Do not model expected failures as panics/exceptions-for-control-flow, and do not swallow bugs into a recoverable path. Rust encodes this split directly: `Result` = recoverable, `panic!` = bug. In exception languages, keep the split by convention: validation → typed exception you catch; invariant break → let it propagate/crash.

## Rust: Result/Option & the anyhow-vs-thiserror Rule

**Decision rule:**

- **`thiserror`** → **libraries** and any code whose caller must *match on* and react to specific error variants. Gives typed, exhaustive enums with `#[from]` conversions.
- **`anyhow`** (or `eyre`) → **application/binary** top layers where you only need to *propagate, add context, and report*. One `anyhow::Error` type, cheap `.context()`, backtraces.
- Mixed is normal: libs export `thiserror` enums; `main`/handlers use `anyhow` and add context as errors bubble up.

```rust
use thiserror::Error;
#[derive(Error, Debug)]
pub enum AppError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("resource not found: {resource_type} id={id}")]
    NotFound { resource_type: String, id: String },
    #[error("database error")]
    Database(#[from] sqlx::Error),   // free `?` conversion, preserves source
}

// anyhow at the app layer: `?` + context to build a cause chain.
use anyhow::{Context, Result};
fn process_order(id: &str) -> Result<Order> {
    let order = fetch_order(id).with_context(|| format!("fetching order {id}"))?;
    validate(&order).context("order validation")?;
    Ok(order)
}
```

Combinators over match ladders:

- `?` propagates and auto-converts via `From`/`#[from]`. `map_err` adapts an error type; `.context()`/`.with_context()` (lazy, use for allocating messages) wrap while preserving `source()`.
- `Option`: `ok_or_else` → `Result`; `?` works on `Option` in `Option`-returning fns. Prefer `.ok_or_else(|| ...)` (lazy) over `.ok_or(expensive())`.
- ⚠ Never `.unwrap()`/`.expect()` on recoverable errors in library/production paths — that converts a handleable error into a crash. `expect("invariant: X")` is acceptable only for genuine bugs where the message documents the invariant.

## Cause Chains — Wrap, Don't Erase

Wrapping adds context; it must **preserve the original cause** so a reader can trace root cause. Every language has the idiom — use it:

| Language | Wrap-with-cause | Inspect chain |
| --- | --- | --- |
| Rust | `.with_context(...)` / `#[source]` / `#[from]` | `err.source()` / `{:?}` |
| Go | `fmt.Errorf("...: %w", err)` | `errors.Is` / `errors.As` / `Unwrap` |
| Python | `raise New(...) from err` | `__cause__` / traceback |
| JS/TS | `new Error(msg, { cause: err })` | `err.cause` |

```go
// Go: %w preserves the chain so errors.Is/As work up the stack.
if err != nil {
    return fmt.Errorf("process order %s: %w", id, err)
}
// caller: if errors.Is(err, sql.ErrNoRows) { ... }
```

```python
# Python: `from err` keeps __cause__; bare `raise New(...)` LOSES the original.
try:
    order = fetch_order(id)
except DatabaseError as e:
    raise ServiceError("failed to process order", code="ORDER_FAILED",
                       details={"order_id": id}) from e     # ← preserve cause
```

⚠ `%v` (Go) or f-string interpolation of an error into a *new* message string does NOT preserve the typed chain — `errors.Is`/`isinstance` checks then break. Use `%w` / `from` / `cause`.

## Anti-Patterns (the ones that actually bite)

- **Swallowing** — empty `catch {}`, bare `except:`, `if err != nil { }`, `let _ = op();`. At minimum log; usually handle or propagate. Bare `except:` also eats `KeyboardInterrupt`/`CancelledError` — always `except Exception` or narrower.
- **Log-and-rethrow** — logging an error then re-throwing it logs the same failure at every frame → noisy duplicate stacks. **Handle OR log-and-swallow OR propagate — pick one.** Rule: log where you *handle* (usually one boundary), propagate everywhere else.
- **Exceptions for control flow** — using try/except to implement normal branching is slow and hides logic; check the condition instead. (Python's EAFP is fine for genuinely exceptional cases, not for expected branches.)
- **Catch-all too early** — a broad catch near the leaf hides the specific error and its recovery opportunity. Catch narrow and near, or broad and at the boundary — not broad and deep.
- **Retrying non-idempotent ops** — see below.
- **Leaking internals** — stack traces / SQL / paths in API responses; see API section.

## Recovery Strategies

Choose by failure kind:

| Strategy | Use when | Caution |
| --- | --- | --- |
| **Retry + backoff + jitter** | Transient AND **idempotent** | Never retry a non-idempotent write without an idempotency key |
| **Fallback** | A degraded alternative exists (cache, default, secondary) | Fallback data must be safe to serve; log the degradation |
| **Circuit breaker** | Repeated failures to a dependency; prevent cascade | Tune threshold/timeout; expose state for observability |
| **Fail fast** | Bad config, missing dep at startup, unrecoverable | Crash cleanly at the boundary, don't limp on |

⚠ **Retry only idempotent operations.** GET/PUT/DELETE are typically idempotent; a raw POST "create" is not — a retried create makes duplicates. Make writes idempotent with a client-supplied idempotency key, or don't retry them. Also: retry only *transient* errors (timeout, 429, 503) — retrying a 400/permission error just wastes time and amplifies load.

**Backoff must have jitter** to avoid the thundering-herd where all clients retry in lockstep and re-synchronize the overload. `delay = min(base * 2^attempt, cap)`; then randomize (full jitter: `random(0, delay)`).

```rust
// Cap + exponential + jitter; retry only on retryable errors.
let mut attempt = 0u32;
loop {
    match op().await {
        Ok(v) => break Ok(v),
        Err(e) if !e.is_retryable() || attempt >= max => break Err(e),
        Err(_) => {
            let backoff = (base_ms * 2u64.pow(attempt)).min(cap_ms);
            let delay = rand::thread_rng().gen_range(0..=backoff);   // full jitter
            tokio::time::sleep(Duration::from_millis(delay)).await;
            attempt += 1;
        }
    }
}
```

```python
# Circuit breaker: CLOSED → (failures ≥ threshold) → OPEN → (after timeout) → HALF_OPEN → CLOSED/OPEN
class CircuitBreaker:
    def __init__(self, threshold=5, recovery_timeout=30.0):
        self.threshold, self.recovery_timeout = threshold, recovery_timeout
        self.state, self.failures, self.opened_at = "closed", 0, 0.0
    def call(self, op):
        if self.state == "open":
            if time.time() - self.opened_at > self.recovery_timeout:
                self.state = "half_open"          # probe with a single call
            else:
                raise CircuitOpenError()
        try:
            r = op(); self.failures = 0; self.state = "closed"; return r
        except Exception:
            self.failures += 1
            if self.failures >= self.threshold:
                self.state, self.opened_at = "open", time.time()
            raise
```

## API Error Responses (RFC 9457 / 7807 problem+json)

Design rules for HTTP APIs:

- **Stable machine-readable code** the client can branch on (`"code": "ORDER_NOT_FOUND"`), independent of the human `detail` message. Never make clients string-match on prose.
- **Never leak internals** — no stack traces, SQL, file paths, or exception class names in 5xx responses. Show `detail` for 4xx (client can act on it), hide it for 5xx.
- Content type `application/problem+json`; fields: `type`, `title`, `status`, `detail`, `instance` (+ your `code` and any safe extensions).
- Map error *kind* → status: validation→400, auth→401, forbidden→403, not-found→404, conflict→409, rate-limit→429, dependency down→503, bug→500.

```python
class ProblemDetails(BaseModel):
    type: str = "about:blank"
    title: str
    status: int
    code: str | None = None
    detail: str | None = None
    instance: str | None = None

@app.exception_handler(AppError)
async def handler(request, error: AppError):
    status = {ValidationError: 400, NotFoundError: 404, ServiceError: 503}.get(type(error), 500)
    problem = ProblemDetails(
        title=type(error).__name__, status=status, code=error.code,
        detail=str(error) if status < 500 else None,     # hide internals on 5xx
        instance=str(request.url.path),
    )
    return JSONResponse(status, problem.model_dump(exclude_none=True),
                        headers={"Content-Type": "application/problem+json"})
```

```rust
// Rust/axum: map variants to status; collapse internal errors to a generic 500.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            AppError::Validation(m)          => (StatusCode::BAD_REQUEST, Some(m)),
            AppError::NotFound { .. }         => (StatusCode::NOT_FOUND, Some(self.to_string())),
            AppError::Database(_)             => (StatusCode::INTERNAL_SERVER_ERROR, None), // never expose
        };
        (status, Json(json!({ "title": status.canonical_reason(),
                              "status": status.as_u16(), "detail": detail }))).into_response()
    }
}
```

## Security-Aware Handling

- **Production vs dev detail** — full error type + backtrace only when `ENVIRONMENT != "production"`; production returns a generic message. Gate on env, not on a debug flag a client can flip.
- **Sanitize before logging** — redact emails, SSNs, card numbers, tokens/passwords from messages *before* they hit logs. An error string built from user input can carry PII or secrets into your log store.
- **Rate-limit error logs** — a hot failure path can flood logs (cost + DoS); throttle per error-key (log first occurrence + a periodic count).
- **Uniform auth errors** — return the same 401/403 for "user not found" and "wrong password" to avoid user enumeration.

```python
SENSITIVE = [r'[\w.%+-]+@[\w.-]+\.\w{2,}', r'\b\d{3}-\d{2}-\d{4}\b',
             r'\b(?:\d{4}[-\s]?){3}\d{4}\b', r'password["\']?\s*[:=]\s*\S+']
def sanitize(msg: str) -> str:
    for p in SENSITIVE: msg = re.sub(p, '[REDACTED]', msg, flags=re.IGNORECASE)
    return msg
```

## Data Pipeline / Batch Errors

Batch jobs need **partial-failure** handling: don't let one bad record kill 10k good ones. Collect failures with their index + payload, route to a dead-letter queue, and surface an error summary — but fail the whole job if the failure *rate* crosses a threshold (a 40% failure rate is a systemic problem, not bad records).

```python
@dataclass
class BatchResult(Generic[T]):
    ok: list[T]; failed: list[dict]; summary: dict[str, int]

def process_batch(items, fn, *, continue_on_error=True) -> BatchResult:
    ok, failed, counts = [], [], {}
    for i, item in enumerate(items):
        try:
            ok.append(fn(item))
        except Exception as e:
            counts[type(e).__name__] = counts.get(type(e).__name__, 0) + 1
            failed.append({"index": i, "item": item, "error": str(e)})
            logging.error("item %d failed: %s", i, e)
            if not continue_on_error: raise
    return BatchResult(ok, failed, counts)
# route failed → dead_letter_queue; alert if len(failed)/len(items) > threshold
```

## Verification Checklist

- [ ] Every failure classified: recoverable (typed error) vs bug (panic/assert) vs fatal (fail-fast) — no expected failures modeled as panics, no bugs swallowed into recoverable paths
- [ ] Rust: `thiserror` for libs / matchable errors, `anyhow` for app layer; no `.unwrap()`/`.expect()` on recoverable errors
- [ ] Error wrapping preserves the cause chain (`%w` / `from` / `{ cause }` / `#[source]`) — `errors.Is`/`isinstance`/`source()` still work at the top
- [ ] No swallowed errors (empty catch, bare `except`, ignored `err`); no log-and-rethrow (log at exactly one boundary)
- [ ] Retries only on transient AND idempotent ops; non-idempotent writes use an idempotency key; backoff has jitter and a cap
- [ ] Circuit breaker / fallback where a dependency failure could cascade; breaker state observable
- [ ] API responses: stable machine `code`, correct status per kind, `application/problem+json`; NO stack traces/SQL/paths in 5xx
- [ ] Secrets/PII sanitized before logging; error logs rate-limited; auth errors uniform (no enumeration)
- [ ] Batch/pipeline: partial failures collected + dead-lettered; job fails if failure rate crosses threshold
- [ ] Exceptions are not used for normal control flow
