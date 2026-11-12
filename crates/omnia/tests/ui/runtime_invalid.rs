mod case_plugins_empty {
    omnia::runtime!({
        plugins: {},
    });
}

mod case_plugins_unknown_key {
    omnia::runtime!({
        plugins: { cache: "store" },
    });
}

mod case_top_level_registries {
    omnia::runtime!({
        registries: "default_registry = \"ghcr.io\"\n",
    });
}

mod case_digest_missing_prefix {
    omnia::runtime!({
        guests: [{ path: "app.wasm", digest: "md5:00" }],
    });
}

mod case_digest_short {
    omnia::runtime!({
        guests: [{ path: "app.wasm", digest: "sha256:abc" }],
    });
}

mod case_digest_non_hex {
    omnia::runtime!({
        guests: [{
            path: "app.wasm",
            digest: "sha256:zz00000000000000000000000000000000000000000000000000000000000000",
        }],
    });
}

mod case_guest_both_sources {
    omnia::runtime!({
        guests: [{ name: "tool", path: "tool.wasm", package: "acme:tool@1.2.3" }],
    });
}

mod case_guest_retired_on_demand {
    omnia::runtime!({
        guests: [{ path: "tool.wasm", on_demand: true }],
    });
}

mod case_guest_retired_wasm_only {
    omnia::runtime!({
        guests: [{ path: "tool.wasm", wasm_only: true }],
    });
}

mod case_computed_guest_path {
    omnia::runtime!({
        guests: [{ path: engine_component_path() }],
    });
}

mod case_guest_path_missing {
    omnia::runtime!({
        guests: [{ name: "api" }],
    });
}

mod case_plugins_beside_manifest {
    omnia::runtime!({
        manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
        plugins: { registries: "default_registry = \"ghcr.io\"\n" },
    });
}

fn main() {}
