//! Blobstore example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_blobstore::{WasiBlobstore, BlobstoreDefault};
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
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
