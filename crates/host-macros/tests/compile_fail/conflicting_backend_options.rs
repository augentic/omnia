omnia_host_macros::runtime!({
    hosts: {
        WasiKeyValue: Filesystem(FilesystemOptions::at("a")),
        WasiBlobstore: Filesystem(FilesystemOptions::at("b")),
    },
});

fn main() {}
