#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some(argument) = std::env::args().nth(1) {
        anyhow::bail!("unsupported argument: {argument}");
    }
    app::bootstrap::run().await
}
