mod case_plugins_empty {
    omnia_host_macros::runtime!({
        plugins: {},
    });
}

mod case_plugins_unknown_key {
    omnia_host_macros::runtime!({
        plugins: { cache: "store" },
    });
}

mod case_top_level_registries {
    omnia_host_macros::runtime!({
        registries: "default_registry = \"ghcr.io\"\n",
    });
}

mod case_digest_missing_prefix {
    omnia_host_macros::runtime!({
        guests: [{ path: "app.wasm", digest: "md5:00" }],
    });
}

mod case_digest_short {
    omnia_host_macros::runtime!({
        guests: [{ path: "app.wasm", digest: "sha256:abc" }],
    });
}

mod case_digest_non_hex {
    omnia_host_macros::runtime!({
        guests: [{
            path: "app.wasm",
            digest: "sha256:zz00000000000000000000000000000000000000000000000000000000000000",
        }],
    });
}

mod case_guest_both_sources {
    omnia_host_macros::runtime!({
        guests: [{ name: "tool", path: "tool.wasm", package: "acme:tool@1.2.3" }],
    });
}

mod case_guest_retired_on_demand {
    omnia_host_macros::runtime!({
        guests: [{ path: "tool.wasm", on_demand: true }],
    });
}

mod case_guest_retired_wasm_only {
    omnia_host_macros::runtime!({
        guests: [{ path: "tool.wasm", wasm_only: true }],
    });
}

mod case_computed_guest_path {
    omnia_host_macros::runtime!({
        guests: [{ path: engine_component_path() }],
    });
}

mod case_guest_path_missing {
    omnia_host_macros::runtime!({
        guests: [{ name: "api" }],
    });
}

mod case_plugins_beside_manifest {
    omnia_host_macros::runtime!({
        manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
        plugins: { registries: "default_registry = \"ghcr.io\"\n" },
    });
}

mod case_env_empty {
    omnia_host_macros::runtime!({
        env: {},
    });
}

mod case_env_unknown_key {
    omnia_host_macros::runtime!({
        env: { pass: ["HOME"] },
    });
}

mod case_grants_unknown_key {
    omnia_host_macros::runtime!({
        guests: [{ path: "app.wasm", grants: { network: ["api"] } }],
    });
}

fn main() {}
