use omnia_core::HostCtx;

pub trait FooClient {}

#[derive(HostCtx)]
pub struct WasiFoo<T>(pub T);

fn main() {}
