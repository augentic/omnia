use omnia_sdk::Provider;

#[derive(Provider)]
struct Unknown {
    #[provider(NotACapability)]
    field: (),
}

fn main() {}
