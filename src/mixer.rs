use anyhow::{Context,Result};
use tokio::net::{UdpSocket,ToSocketAddrs};
use tokio::time::{self, Duration, Instant};
use tokio::sync::Mutex;
use rosc::{OscMessage, OscPacket, OscType};
use std::collections::VecDeque;

#[derive(Debug)]
struct Channel {
    fader: f32,
    on: bool,
    compressed: bool,
}

#[derive(Debug)]
pub struct MixerState{
    lr_fader: f32,
    lr_on: bool,
    channels: [Channel;16],
}

const MIN_METER_VAL: f32 = -128.0;
#[derive(Debug)]
pub struct MeterHistory{
    history: VecDeque<f32>,
    max_history_size: usize,

    current_minute_peak: f32,
    last_reset: Instant,
    last_poll: Instant,
}

impl MeterHistory {
    pub fn new(minutes: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(minutes),
            max_history_size: minutes,
            current_minute_peak: MIN_METER_VAL, // Start at silence
            last_reset: Instant::now(),
            last_poll: Instant::now(),
        }
    }

    pub fn new_data(&mut self, meter: f32) {
        self.last_poll = Instant::now();
        if meter > self.current_minute_peak {
            self.current_minute_peak = meter
        }

        if self.last_reset.elapsed() >= Duration::from_secs(60) {
            if self.history.len() >= self.max_history_size {
                self.history.pop_front();
            }
            self.history.push_back(self.current_minute_peak);

            // Reset for the new minute
            self.current_minute_peak = MIN_METER_VAL;
            self.last_reset = Instant::now();
        }
    }
}

#[derive(Debug)]
pub struct Mixer {
    socket: UdpSocket,
    state: Mutex<MixerState>,
    meter: Mutex<MeterHistory>,
}

impl Mixer {
    pub async fn new<T: ToSocketAddrs>(addr: T) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(addr).await?;
        let lr_fader = recv_float_osc(&socket, "/lr/mix/fader").await?;
        let lr_on = recv_bool_osc(&socket, "/lr/mix/on").await?;

        let mut channels: [Option<Channel>; 16] = std::array::from_fn(|_| None);

        for i in 0..16 {
            channels[i] = Some(Channel::new(&socket, i).await?);
        }

        let channels = channels.map(|c| c.unwrap());
        let state = Mutex::new(MixerState{lr_fader, lr_on, channels});
        let meter = Mutex::new(MeterHistory::new(30));

        Ok(Self{socket, state, meter})
    }

    async fn update(&self, addr: &str, args: Vec<OscType>) -> Result<()> {
        let mut addr_by_parts = addr.split('/');
        addr_by_parts.next(); // Let's get rid of '/'
        let chan = addr_by_parts.next().unwrap();
        if chan == "ch" {
            let chan_num: usize = addr_by_parts.next().unwrap().parse()?;
            if chan_num >= 16 { return Ok(()) };
            let mut state = self.state.lock().await;
            state.channels[chan_num].update(&addr_by_parts.collect::<Vec<_>>().join("/"), args)?;
        } else if chan == "lr" {
            if addr_by_parts.next().unwrap() != "mix" { return Ok(()) };
            let mut state = self.state.lock().await;
            match addr_by_parts.next().unwrap() {
                "fader" => state.lr_fader = parse_float(args)?,
                "on" => state.lr_on = parse_bool(args)?,
                _ => ()
            }
        }
        Ok(())
    }

    async fn update_meter(&self, args: Vec<OscType>) -> Result<()> {
        let first_arg = args.get(0).context("Missing arg in /meters/0")?;

        let bytes = match first_arg {
            OscType::Blob(b) => b,
            _ => anyhow::bail!("Argument is not a blob"),
        };
        let meter_data = &bytes[4..]; 
        let values: Vec<f32> = meter_data
            .chunks_exact(2)
            .map(|chunk| {
                // XR18 uses Little Endian for these 16-bit values
                let raw = i16::from_le_bytes([chunk[0], chunk[1]]);
                raw as f32 / 256.0
            })
            .collect();
        let val = values
            .get(4..5).context("Wrong size")?
            .iter()
            .copied()
            .max_by(|a, b| a.total_cmp(b))
            .unwrap_or(MIN_METER_VAL);

        let mut meter_history = self.meter.lock().await;
        meter_history.new_data(val);
        Ok(())
    }


    pub async fn heartbeat_loop(&self) -> Result<()> {
        let mut interval = time::interval(Duration::from_secs(8));
        loop {
            interval.tick().await;
            send_osc(&self.socket, "/xremote", vec![])
                .await?;
            send_osc(
                &self.socket,
                "/meters",
                vec![OscType::String("/meters/0".to_string()), OscType::Int(31)]
            ).await?;

            let meter = self.meter.lock().await;
            let since_last_poll = Instant::now() - meter.last_poll;
            if since_last_poll > Duration::from_secs(30) {
                log::warn!("Mixer is not connected!")
            }
        }
    }

    pub async fn update_loop(&self) -> Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        let len = self.socket.recv(&mut buf).await?;
        if let Ok((_, OscPacket::Message(msg))) = rosc::decoder::decode_udp(&buf[..len]) {
            match msg.addr.as_str() {
                "/meters/0" => {
                    self.update_meter(msg.args).await?;
                },
                addr => {
                    self.update(addr, msg.args).await?;
                }
            }
        }
    }
}

    async fn lr_fader_db(&self) -> f64 {
        let f = self.state.lock().await.lr_fader as f64;

        if f >= 0.5 {
            40.0 * f - 30.0
        } else if f >= 0.25 {
            80.0 * f - 50.0
        } else if f >= 0.0625 {
            160.0 * f - 70.0
        } else if f > 0.0 {
            480.0 * f - 90.0
        } else {
            return f64::NEG_INFINITY;
        }
    }

    async fn inc_fader(&self, db: f64) -> Result<()> {
        let val = db_to_float(self.lr_fader_db().await + db);
        send_osc(&self.socket, "/lr/mix/fader", vec![OscType::Float(val)]).await
    }

    async fn toggle_mute(&self) -> Result<()> {
        let val = if self.state.lock().await.lr_on { 0 } else { 1 };
        send_osc(&self.socket, "/lr/mix/on", vec![OscType::Int(val)]).await
    }

    async fn toggle_comp(&self, chan: usize) -> Result<()> {
        let state = self.state.lock().await;
        let channel = state
            .channels.get(chan).context("Wrong channel number")?;
        let val = if channel.compressed { 0 } else { 1 };

        let addr = format!("/ch/{:02}/dyn/on", chan);

        send_osc(&self.socket, &addr, vec![OscType::Int(val)]).await
    }

    pub async fn exec_cmd(&self, cmd: &str, val: &[&str]) -> Result<()> {
        match cmd {
            "inc-vol" => {
                let inc_vol: f64 = val[0].parse()?;
                self.inc_fader(inc_vol).await?;
            },
            "toggle-mute" => {
                self.toggle_mute().await?;
            },
            "toggle-comp" => {
                let chan: usize = val[0].parse()?;
                self.toggle_comp(chan).await?;
            },
            _ => println!("Unknown command {}", cmd)
        }
        Ok(())
    }

}

impl Channel {
    async fn new(socket: &UdpSocket, chan: usize) -> Result<Self> {
        let addr_start = format!("/ch/{:02}", chan+1);
        let fader = recv_float_osc(socket, &format!("{addr_start}/mix/fader")).await?;
        let on = recv_bool_osc(socket, &format!("{addr_start}/mix/on")).await?;
        let compressed = recv_bool_osc(socket, &format!("{addr_start}/dyn/on")).await?;

        Ok(Self{fader, on, compressed})
    }


    fn update(&mut self, addr: &str, args: Vec<OscType>) -> Result<()> {
        let mut addr_by_parts = addr.split('/');
        match addr_by_parts.next().unwrap() {
            "mix" => match addr_by_parts.next().unwrap() {
                "fader" => self.fader = parse_float(args)?,
                "on" => self.on = parse_bool(args)?,
                _ => ()
            },
            "dyn" => match addr_by_parts.next().unwrap() {
                "on" => self.compressed = parse_bool(args)?,
                _ => ()
            },
            _ => ()
        }
        Ok(())
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


async fn send_osc(
    socket: &UdpSocket,
    addr: &str,
    args: Vec<OscType>
) -> Result<()> {
    let msg = OscPacket::Message(
        OscMessage { addr: addr.into(), args }
    );
    let packet =
        rosc::encoder::encode(&msg)
        .context("OSC encoding failed")?;
    socket.send(&packet).await.context("UDP send failed")?;
    Ok(())
}

async fn recv_osc(socket: &UdpSocket) -> Result<OscMessage> {
    let mut buf = [0u8; 4096];
    let len = socket.recv(&mut buf).await?;
    let packet = rosc::decoder::decode_udp(&buf[..len])
        .context("Failed to decode UDP packet")?;

    match packet {
        (_, OscPacket::Message(msg)) => Ok(msg),
        _ => anyhow::bail!("Not a message")
    }
}

async fn recv_one_var_osc(socket: &UdpSocket, addr: &str) -> Result<OscType> {
    send_osc(socket, addr, vec![]).await?;
    let msg = recv_osc(socket).await?;
    if msg.addr != addr {
        anyhow::bail!("Wrong message");
    }
    parse_one_arg(msg.args)
}

fn parse_one_arg(args: Vec<OscType>) -> Result<OscType> {
    if args.len() != 1 {
        anyhow::bail!("Wrong length");
    }
    args
    .first()
    .context("Wrong length")
    .cloned()
}

fn parse_bool(args: Vec<OscType>) -> Result<bool> {
    let arg = parse_one_arg(args)?;
    Ok(arg.int().context("Wrong type")? == 1)
}

fn parse_float(args: Vec<OscType>) -> Result<f32> {
    let arg = parse_one_arg(args)?;
    arg.float().context("Wrong type")
}


async fn recv_bool_osc(socket: &UdpSocket, addr: &str) -> Result<bool> {
    let recval = recv_one_var_osc(socket, addr).await?;
    Ok(recval.int().context("Wrong type")? == 1)
}

async fn recv_float_osc(socket: &UdpSocket, addr: &str) -> Result<f32> {
    let recval = recv_one_var_osc(socket, addr).await?;
    recval.float().context("Wrong type")
}


