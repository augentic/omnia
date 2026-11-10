use std::fmt::Debug;

use omnia_core::{Backend, Connect, HostCtx, NoOptions};

pub trait FooClient: Debug + Send + Sync + 'static {}

#[derive(Debug, HostCtx)]
#[host_ctx(client = FooClient)]
pub struct WasiFoo;

omnia_core::capability!(WasiFoo, "omnia:foo", FooClient);

#[derive(Clone, Debug)]
pub struct Memory;

impl Connect for Memory {
    type ConnectOptions = NoOptions;

    async fn connect_with(_: NoOptions) -> omnia_core::anyhow::Result<Self> {
        Ok(Self)
    }
}

impl Backend for Memory {
    type Hosts = (WasiFoo,);
}

fn main() {}
