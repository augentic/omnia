omnia_host_macros::runtime!({
    guests: [
        { id: "router", source: "router.wasm", dispatch: ["omnia:link/echo"] },
    ],
});

fn main() {}
