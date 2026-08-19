#[omnia_guest_macros::operation]
async fn handler<P>(input: &Message, context: CallContext<'_, P>) -> Result<()> {
    Ok(())
}

fn main() {}
