omnia_host_macros::runtime!({
    plugins: {
        interfaces: ["omnia:shared/log"],
        acquire: MountAcquire,
        locations: [{ registry: "ghcr.io" }],
    },
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
