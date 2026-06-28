use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore};
use omnia_wasi_http::{HttpDefault, WasiHttp};

warp::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiBlobstore: BlobstoreDefault,
    }
});

#[derive(WasiHttpView)]
#[wasi_http(field = "http")]
pub struct StoreCtx {
    pub table: ResourceTable,
    pub wasi: WasiCtx,
    // #(pub #store_ctx_fields,)*
}
