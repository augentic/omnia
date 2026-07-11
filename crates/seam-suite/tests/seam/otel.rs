//! `wasi:otel` seam: an instrumented guest handler's telemetry crosses the
//! boundary into the capturing host exporter.

use anyhow::Result;
use omnia_testkit::http;

use crate::fixture;

#[test]
fn guest_telemetry() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        let response = http::post_json(&fx.runtime, "/otel", r#"{"trace":"me"}"#).await?;
        assert!(response.status().is_success(), "instrumented guest handles the request");

        // Metrics flush deterministically when the guest's telemetry guard drops
        // at the end of the handler, so they are the reliable proof that
        // instrumentation crossed into the host exporter. (Spans ride a separate
        // sampled batch flush, counted here only for diagnostics.)
        let (spans, metrics) = {
            let captured = fx.otel.captured.lock().expect("otel capture lock");
            (captured.spans, captured.metrics)
        };
        assert!(
            metrics > 0,
            "the guest's metrics reached the host exporter (spans={spans}, metrics={metrics})"
        );

        Ok(())
    })
}
