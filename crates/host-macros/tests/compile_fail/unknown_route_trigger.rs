omnia_host_macros::runtime!({
    guests: [
        { id: "api", source: "api.wasm", routes: { grpc: ["/"] } },
    ],
});

fn main() {}
