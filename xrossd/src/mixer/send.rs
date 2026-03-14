use anyhow::{Context,Result};
use xrossd_core::field::send_osc;
use rosc::OscType;
use crate::mixer::Mixer;

impl Mixer {
    async fn inc_fader(&self, db: f64) -> Result<()> {
        if let Some(ready) = self.ready().await {
            let db = float_to_db(ready.lr_fader.val) + db;
            let val = db_to_float(db);
            send_osc(&self.socket, "/lr/mix/on", vec![OscType::Float(val)]).await
        } else {
            Ok(())
        }
    }

    async fn toggle_mute(&self) -> Result<()> {
        if let Some(ready) = self.ready().await {
            let val = if ready.lr_on.val { 0 } else { 1 };
            send_osc(&self.socket, "/lr/mix/on", vec![OscType::Int(val)]).await
        } else {
            Ok(())
        }
    }

    async fn toggle_comp(&self, chan: usize) -> Result<()> {
        if let Some(ready) = self.ready().await {
            let channel = ready
                .channels.get(chan).context("Wrong channel number")?;
            let val = if channel.compressed.val { 0 } else { 1 };
            let addr = format!("/ch/{:02}/dyn/on", chan);
            send_osc(&self.socket, &addr, vec![OscType::Int(val)]).await
        } else {
            Ok(())
        }
    }

    pub async fn exec_cmd(&self, cmd: &str, val: &[&str]) -> Result<()> {
        match cmd {
            "inc-vol" => {
                let inc_vol: f64 = val[0].parse()?;
                self.inc_fader(inc_vol).await
            },
            "toggle-mute" => {
                self.toggle_mute().await
            },
            "toggle-comp" => {
                let chan: usize = val[0].parse()?;
                self.toggle_comp(chan).await
            },
            _ => {
                println!("Unknown command {}", cmd);
                Ok(())
            }
        }
    }
}


fn db_to_float(db: f64) -> f32 {
    let res = if db <= -90.0 { 0.0 }
    else if db < -60.0 { (db + 90.0) / 480.0 }
    else if db < -30.0 { (db + 70.0) / 160.0 }
    else if db < -10.0 { (db + 50.0) / 80.0 }
    else if db <= 10.0 { (db + 30.0) / 40.0 }
    else { 1.0 };
    res as f32
}

fn float_to_db(float: f32) -> f64 {
    let f = float as f64;
    if f >= 0.5 {
        40.0 * f - 30.0
    } else if f >= 0.25 {
        80.0 * f - 50.0
    } else if f >= 0.0625 {
        160.0 * f - 70.0
    } else if f > 0.0 {
        480.0 * f - 90.0
    } else {
        f64::NEG_INFINITY
    }
}
