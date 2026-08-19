#[omnia_guest_macros::operation(name = "custom_span")]
async fn handler<P>(input: Message, context: CallContext<'_, P>) -> Result<()> {
    Ok(())
}

fn main() {}
