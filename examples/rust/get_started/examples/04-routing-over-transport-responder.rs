// This node starts a tcp listener and an echoer worker.
// It then runs forever waiting for messages.

use hello_ockam::Echoer;
use ockam::access_control::{AllowFromTransport, AllowToTransport};
use ockam::{Context, Mailboxes, Result, TcpTransport, WorkerBuilder, TCP};
use std::sync::Arc;

#[ockam::node]
async fn main(ctx: Context) -> Result<()> {
    // Initialize the TCP Transport.
    let tcp = TcpTransport::create(&ctx).await?;

    // Create a TCP listener and wait for incoming connections.
    // Use port 4000, unless otherwise specified by command line argument.
    let port = std::env::args().nth(1).unwrap_or_else(|| "4000".to_string());
    tcp.listen(format!("127.0.0.1:{port}")).await?;

    // Create an echoer worker
    WorkerBuilder::with_mailboxes(
        Mailboxes::main(
            "echoer",
            Arc::new(AllowFromTransport::single(TCP)),
            Arc::new(AllowToTransport::single(TCP)),
        ),
        Echoer,
    )
    .start(&ctx)
    .await?;

    // Don't call ctx.stop() here so this node runs forever.
    Ok(())
}
