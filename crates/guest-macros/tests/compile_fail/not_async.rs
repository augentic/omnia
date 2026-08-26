#[omnia_guest_macros::operation]
fn handler<P>(input: Message, context: Context<'_, P>) -> Result<()> {
    Ok(())
}

fn main() {}
