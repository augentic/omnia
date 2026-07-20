omnia_host_macros::runtime!({
    config: "omnia.toml",
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
