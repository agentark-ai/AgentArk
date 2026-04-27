use anyhow::Result;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    agentark::run().await
}
