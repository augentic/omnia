mod case_unknown_key {
    #[derive(omnia_host_macros::FromEnv)]
    struct Options {
        #[env(form = "PORT")]
        port: u16,
    }
}

mod case_enum {
    #[derive(omnia_host_macros::FromEnv)]
    enum Mode {
        Fast,
    }
}

mod case_default_on_option {
    #[derive(omnia_host_macros::FromEnv)]
    struct Options {
        #[env(from = "PORT", default = "8080")]
        port: Option<u16>,
    }
}

mod case_no_env {
    #[derive(omnia_host_macros::FromEnv)]
    struct Options {
        port: u16,
    }
}

fn main() {}
