//! The service entry point.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Both `ring` and `aws-lc-rs` end up in this binary — different
    // dependencies pull different rustls backends — so rustls refuses to guess
    // and panics on the first handshake. The panic lands on stderr inside a
    // spawned task, which is why it cost a day to find: recognition simply
    // stopped existing, with every structured log silent about it.
    //
    // Choosing here, once, is the fix the panic message asks for.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a rustls crypto provider was already installed"))?;

    if let Some(argument) = std::env::args().nth(1) {
        anyhow::bail!("unsupported argument: {argument}");
    }
    app::bootstrap::run().await
}
