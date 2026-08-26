#[omnia_guest_macros::handler]
async fn handler<P>(input: Message, context: Context<'_, P>) -> u32 {
    0
}

fn main() {}
