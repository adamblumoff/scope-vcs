#[tokio::main]
async fn main() -> anyhow::Result<()> {
    api::smoke_seed::run().await
}
