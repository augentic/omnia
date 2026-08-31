omnia_host_macros::runtime!({
    plugins: {
        interfaces: ["omnia:shared/log"],
        locations: [],
    },
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
