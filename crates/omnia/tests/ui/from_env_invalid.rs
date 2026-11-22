mod case_unknown_key {
    #[derive(omnia::FromEnv)]
    #[env(crate = omnia)]
    struct Options {
        #[env(form = "PORT")]
        port: u16,
    }
}

mod case_enum {
    #[derive(omnia::FromEnv)]
    #[env(crate = omnia)]
    enum Mode {
        Fast,
    }
}

mod case_default_on_option {
    #[derive(omnia::FromEnv)]
    #[env(crate = omnia)]
    struct Options {
        #[env(from = "PORT", default = "8080")]
        port: Option<u16>,
    }
}

mod case_no_env {
    #[derive(omnia::FromEnv)]
    #[env(crate = omnia)]
    struct Options {
        port: u16,
    }
}

fn main() {}
