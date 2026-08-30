omnia_host_macros::runtime!({
    plugins: ["omnia:shared/log"],
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
