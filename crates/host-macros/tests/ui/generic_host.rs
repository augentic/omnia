use omnia_host_macros::HostCtx;

pub trait FooClient {}

#[derive(HostCtx)]
pub struct WasiFoo<T>(pub T);

fn main() {}
