omnia_host_macros::runtime!({
    plugins: {
        interfaces: ["omnia:shared/log"],
        acquire: MountAcquire,
    },
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
