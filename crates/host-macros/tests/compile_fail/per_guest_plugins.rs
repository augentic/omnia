omnia_host_macros::runtime!({
    guests: [
        { id: "router", source: "router.wasm", plugins: ["omnia:link/echo"] },
    ],
});

fn main() {}
