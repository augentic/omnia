omnia_host_macros::runtime!({
    dispatch: ["omnia:shared/log"],
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
