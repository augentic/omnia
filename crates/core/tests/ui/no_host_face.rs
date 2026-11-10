use std::fmt::Debug;

use omnia_core::{Backends, Link};

pub trait FooClient: Debug + Send + Sync + 'static {}

#[derive(Debug)]
pub struct WasiFoo;

omnia_core::capability!(WasiFoo, "omnia:foo", FooClient);

fn link<B: Backends>()
where
    (WasiFoo,): Link<B>,
{
}

fn main() {
    link::<()>();
}
