omnia_host_macros::runtime!({
    guests: [
        { id: "app", source: "app.wasm" },
    ],
    command_guest: "app",
});

fn main() {}
