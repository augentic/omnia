struct Provider;
struct Message;

omnia_guest_macros::guest!({
    owner: "examples",
    provider: Provider,
    messaging: [
        "9-lives.v1": Message,
    ],
});

fn main() {}
