struct Provider;
struct Req;
struct Reply;

omnia_guest_macros::guest!({
    owner: "examples",
    provider: Provider,
    http: [
        "/greet": post(Req with_query, Reply),
    ],
});

fn main() {}
