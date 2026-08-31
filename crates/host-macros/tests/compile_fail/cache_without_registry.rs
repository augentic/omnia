omnia_host_macros::runtime!({
    plugins: {
        interfaces: ["omnia:shared/log"],
        locations: [{ name: ".", path: "adapters" }],
        cache: PluginCache,
    },
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
