use xrossd::mixer::Mixer;
use xrossd::config::Config;
use anyhow::{Context,Result};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::io::{AsyncBufReadExt,BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    let conf = Config::load()?;

    let _ = std::fs::remove_file(&conf.socket_path);
    let uds_listener =
        UnixListener::bind(&conf.socket_path).context("Could not bind Unix socket")?;

    let mixer = Arc::new(Mixer::new(conf.mixer_addr).await?);

    println!("{:?}", mixer);

    println!("Daemon active");

    tokio::select! {
        res = mixer.heartbeat_loop() => res?,
        res = mixer.update_loop() => res?,
        res = cli_handler(uds_listener, Arc::clone(&mixer)) => res?,
    }

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
                    let _ = mixer.exec_cmd(cmd, &vals).await;
                }
                line.clear();

            }
        });
    }
}

