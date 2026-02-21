//! WebSocket example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};
        use omnia_wasi_websocket::{WasiWebSocket, WebSocketDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiWebSocket: WebSocketDefault,
            }
        });
    } else {
        fn main() {}
    }
}
