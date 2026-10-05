//! Blobstore example runtime.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore};
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiBlobstore: BlobstoreDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
