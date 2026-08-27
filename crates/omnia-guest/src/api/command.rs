//! WASI CLI glue for guest command entrypoints.

use std::future::Future;
use std::io::Write as _;

/// Execute a guest command at the WASI CLI boundary.
///
/// Initializes guest telemetry, awaits `run`, and flushes telemetry and
/// stdout. The guest writes its own output; a non-zero status then exits
/// with that exact code.
///
/// # Errors
///
/// Returns `Ok(())` when `run` succeeds. A non-zero status is reported
/// through `wasi:cli/exit` and does not return.
#[expect(clippy::result_unit_err, reason = "matches the wasi:cli/run contract")]
pub async fn execute_wasi(run: impl Future<Output = Result<(), u8>>) -> Result<(), ()> {
    let _guard = omnia_wasi_otel::init();
    let result = run.await;
    // `exit-with-code` does not return (analogous to a trap), so no
    // `Drop` runs past it: flush telemetry as soon as the run completes.
    omnia_wasi_otel::shutdown();
    // Rust's exit-time stdout flush never runs here (`exit-with-code` traps
    // out, and a reactor export has no `main`), so flush the line buffer
    // explicitly; stderr is unbuffered by contract.
    let _ = std::io::stdout().flush();
    if let Err(code) = result
        && code != 0
    {
        wasip3::cli::exit::exit_with_code(code);
    }
    Ok(())
}
