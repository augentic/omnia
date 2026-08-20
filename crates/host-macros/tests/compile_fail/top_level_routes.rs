omnia_host_macros::runtime!({
    guests: [
        { id: "api", source: "api.wasm" },
    ],
    routes: {
        http: [{ prefix: "/", guest: "api" }],
    },
});

fn main() {}
