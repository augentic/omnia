#[omnia_guest_macros::handler]
async fn handler<P>(input: &Message, context: Context<'_, P>) -> Result<()> {
    Ok(())
}

fn main() {}
