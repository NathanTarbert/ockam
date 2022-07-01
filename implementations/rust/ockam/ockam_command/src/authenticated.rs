use crate::util::embedded_node;
use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use ockam::{Context, TcpTransport};
use ockam_api::auth;
use ockam_multiaddr::MultiAddr;

#[derive(Clone, Debug, Args)]
pub struct AuthenticatedCommand {
    #[clap(subcommand)]
    subcommand: AuthenticatedSubcommand,
}

#[derive(Clone, Debug, Subcommand)]
pub enum AuthenticatedSubcommand {
    /// Get attribute value.
    Get {
        /// Address to connect to.
        #[clap(long)]
        addr: MultiAddr,

        /// Subject identifier
        #[clap(long, validator(non_empty))]
        id: String,

        /// Attribute key.
        #[clap(validator(non_empty))]
        key: String,
    }
}

impl AuthenticatedCommand {
    pub fn run(c: AuthenticatedCommand) {
        embedded_node(run_impl, c.subcommand)
    }
}

async fn run_impl(mut ctx: Context, cmd: AuthenticatedSubcommand) -> anyhow::Result<()> {
    TcpTransport::create(&ctx).await?;
    match &cmd {
        AuthenticatedSubcommand::Get { addr, id, key } => {
            let mut c = client(addr, &ctx).await?;
            let val = c.get(id, key).await?;
            println!("{val:?}")
        }
    }
    ctx.stop().await?;
    Ok(())
}

async fn client(addr: &MultiAddr, ctx: &Context) -> Result<auth::Client> {
    let to = ockam_api::multiaddr_to_route(addr)
        .ok_or_else(|| anyhow!("failed to parse address: {addr}"))?;
    let cl = auth::Client::new(to, ctx).await?;
    Ok(cl)
}

fn non_empty(arg: &str) -> Result<(), String> {
    if arg.is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(())
}
