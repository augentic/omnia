omnia_host_macros::runtime!({
    mode: command,
    guests: [
        { id: "app", source: "app.wasm", command: true },
        { id: "other", source: "other.wasm", command: true },
    ],
});

fn main() {}
