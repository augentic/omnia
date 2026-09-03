omnia_host_macros::runtime!({
    plugins: {
        interfaces: ["omnia:shared/log"],
        locations: [{ registry: "ghcr.io" }],
        cache: PluginCache,
    },
    guests: [
        { id: "api", source: "api.wasm" },
    ],
});

fn main() {}
