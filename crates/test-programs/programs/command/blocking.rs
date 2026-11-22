//! A command that blocks on the host while a task it spawned waits on the
//! host too: `wait` spawns a one-millisecond `wait-for`, yields so the wait is
//! in flight, blocks in `std::thread::sleep`, then joins the task.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::{Error, Handlers, Router, command};
use wasip3::clocks::monotonic_clock;

async fn wait((): ()) -> Result<(), Error> {
    // spawn a host wait and yield, so the executor puts it in flight
    let waiting = wit_bindgen::spawn_local(monotonic_clock::wait_for(1_000_000));
    wit_bindgen::yield_async().await;

    // block on WASI 0.3's async `wait-for`, parking this task's fiber
    std::thread::sleep(Duration::from_millis(5));

    waiting.await.expect("the spawned wait completes");
    Ok(())
}

fn command_router() -> Router {
    Handlers::new().command("wait", wait).into_router().name("blocking")
}

command::export!(command_router);
