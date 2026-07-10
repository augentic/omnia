struct Provider;
struct Req;
struct Reply;

omnia_guest_macros::guest!({
    owner: "examples",
    provider: Provider,
    http: [
        "/greet": get(Req, Reply),
        "/greet": get(Req, Reply),
    ],
});

fn main() {}
