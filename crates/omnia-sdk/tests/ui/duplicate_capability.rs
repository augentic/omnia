use omnia_sdk::Provider;

#[derive(Provider)]
struct Duplicate {
    #[provider(Config)]
    first: (),
    #[provider(Config)]
    second: (),
}

fn main() {}
