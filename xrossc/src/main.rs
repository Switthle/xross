use anyhow::{Context, Result}; // Result now defaults to anyhow::Result
use tokio::net::UnixStream;
use tokio::io::AsyncWriteExt;
use std::path::PathBuf;
use clap::Parser;
use xross_core::constants::DEFAULT_SOCKET;

#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value = DEFAULT_SOCKET)]
    socket: PathBuf,

    /// Capture all remaining arguments as the command
    #[arg(required = true, num_args = 1..)]
    command: Vec<String>,
}


#[tokio::main]
async fn main() -> Result<()> {
    // 1. Get path (using your logic from before)
    let args = Cli::parse();
    let socket_path = args.socket;

    // 2. Connect with helpful error context
    let mut stream = UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!("Failed to connect to socket at {:?}", socket_path))?;

    // 3. Send command
    let command = args.command.join(" ");
    stream.write_all(command.as_bytes())
        .await
        .context("Failed to write command to socket")?;

    println!("Command sent successfully.");
    Ok(())
}
