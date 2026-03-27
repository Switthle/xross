use anyhow::Result;
use crate::mixer::Mixer;

impl Mixer {
    pub async fn exec_cmd(&self, cmd: &str, val: &[&str]) -> Result<()> {
        if cmd == "reset-history" {
            self.reset_history().await;
            return Ok(());
        }
        if let Some(ready) = self.ready().await {
            match cmd {
                "inc-vol" => {
                    let inc_vol: f64 = val[0].parse()?;
                    ready.inc_db_lr_fader(&self.socket, inc_vol).await
                },
                "toggle-mute" => {
                    ready.toggle_lr_on(&self.socket).await
                },
                "toggle-comp" => {
                    let chan: usize = val[0].parse()?;
                    ready.toggle_chan_compressed(&self.socket, chan).await
                },
                _ => {
                    println!("Unknown command {}", cmd);
                    Ok(())
                }
            }
        } else {
            anyhow::bail!("Command received while mixer not initialized");
        }
    }
}


