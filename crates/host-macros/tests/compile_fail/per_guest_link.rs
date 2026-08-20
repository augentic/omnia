omnia_host_macros::runtime!({
    guests: [
        { id: "router", source: "router.wasm", link: ["omnia:link/echo"] },
    ],
});

fn main() {}
