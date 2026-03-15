use xrossd::mixer::Mixer;
use xrossd::config::Config;
use anyhow::{Context,Result};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::io::{AsyncBufReadExt,BufReader};
use tokio::signal;
use env_logger::Env;
use xrossd_core::errors::LogEntryExt;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let conf = Config::load()?;

    let mixer = Mixer::new(conf.mixer_addr, conf.timeout).await;

    let _ = std::fs::remove_file(&conf.socket_path);
    let uds_listener =
        UnixListener::bind(&conf.socket_path).context("Could not bind Unix socket")?;

    tokio::select! {
        res = cli_handler(uds_listener, Arc::clone(&mixer)) => res?,
        _ = signal::ctrl_c() => {
            log::info!("Shutdown signal received. Cleaning up...");
        },
    }

    let _ = std::fs::remove_file(&conf.socket_path);
    let _ = mixer.shutdown().await;
    return Ok(())
}
async fn cli_handler(
    listener: UnixListener,
    mixer: Arc<Mixer>
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let mixer = Arc::clone(&mixer);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let cmd = line.trim();
                let mut cmd_iter = cmd.split_whitespace();
                if let Some(cmd) = cmd_iter.next() {
                    let vals: Vec<&str> = cmd_iter.collect();
                    let _ = mixer.exec_cmd(cmd, &vals).await.log_err();
                }
                line.clear();

            }
        });
    }
}
