#[omnia_guest_macros::handler(name = "custom_span")]
async fn handler<P>(input: Message, context: Context<'_, P>) -> Result<()> {
    Ok(())
}

fn main() {}
