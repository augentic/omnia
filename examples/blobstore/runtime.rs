//! Blobstore example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_blobstore::{WasiBlobstore, BlobstoreDefault};
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};

        qwasr::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiBlobstore: BlobstoreDefault,
            }
        });
    } else {
        fn main() {}
    }
}
