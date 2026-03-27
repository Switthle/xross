pub mod send;
use std::process::Command;
use std::iter;
use anyhow::{Context,Result};
use tokio::net::{UdpSocket,ToSocketAddrs};
use tokio::time::{self, Duration, Instant};
use tokio::sync::{Mutex,MutexGuard,MappedMutexGuard};
use tokio_util::sync::CancellationToken;
use std::collections::VecDeque;
use std::sync::Arc;
use std::mem;
use xrossd_macro::osc_state;
use xrossd_core::field::{send_osc, parse_one_arg};
use xrossd_core::errors::LogEntryExt;
use tokio::task::JoinSet;
use rosc::{OscPacket, OscType};
use crate::config::ConfigTimeout;

#[osc_state(num_chans=16,chan_prefix="/ch")]
struct MixerState {
    #[address("/lr/mix/fader")]
    lr_fader: f32,
    
    #[address("/lr/mix/on")]
    lr_on: bool,

    #[per_chan]
    #[address("/mix/fader")]
    fader: f32,

    #[per_chan]
    #[address("/mix/on")]
    on: bool,

    #[per_chan]
    #[address("/dyn/on")]
    compressed: bool,
}

const MIN_METER_VAL: f32 = -128.0;
#[derive(Debug)]
pub enum Timeout{
    TimedOut,
    Running,
}

#[derive(Debug)]
pub struct MeterHistory{
    config: Option<ConfigTimeout>,
    status: Timeout,
    history: VecDeque<f32>,
    max_history_size: usize,

    current_minute_peak: f32,
    last_poll: Instant,
}

impl MeterHistory {
    pub fn new(config: Option<ConfigTimeout>) -> Self {
        let minutes = config.as_ref().map_or(0, |c| c.after_mins);
        Self {
            config: config,
            status: Timeout::Running,
            history: VecDeque::with_capacity(minutes),
            max_history_size: minutes,
            current_minute_peak: MIN_METER_VAL, // Start at silence
            last_poll: Instant::now() - Duration::from_hours(1),
        }
    }

    pub fn new_data(&mut self, meter: f32) {
        self.last_poll = Instant::now();
        if meter > self.current_minute_peak {
            self.current_minute_peak = meter
        }
    }

    pub fn push_data(&mut self) {
        if self.has_history() {
            if self.history.len() >= self.max_history_size {
                self.history.pop_front();
            }
            self.history.push_back(self.current_minute_peak);
        }

        // Reset for the new minute
        self.current_minute_peak = MIN_METER_VAL;
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }

    pub fn has_history(&self) -> bool {
        return self.max_history_size > 0;
    }

    pub fn peak(&self) -> Option<f32> {
        self
            .history
            .iter()
            .chain(iter::once(&self.current_minute_peak))
            .max_by(|a,b| a.total_cmp(b))
            .copied()
    }
}

#[derive(Debug)]
pub struct Mixer {
    socket: UdpSocket,
    state: Mutex<MixerState>,
    meter: Mutex<MeterHistory>,

    cancel: CancellationToken,
    jobs: Mutex<JoinSet<()>>
}

impl Mixer {
    pub async fn new<T: ToSocketAddrs>(addr: T, timeout: Option<ConfigTimeout>) -> Arc<Self> {
        log::info!("Mixer initializing...");
        let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        socket.connect(addr).await.unwrap();
        let mixer = Arc::new(
            Self {
                socket,
                state: Mutex::new(MixerState::Disconnected),
                meter: Mutex::new(MeterHistory::new(timeout)),
                cancel: CancellationToken::new(),
                jobs: Mutex::new(JoinSet::new()),
            }
        );

        {
            let mut jobs = mixer.jobs.lock().await;
            let mixer_shared = Arc::clone(&mixer);
            let mixer_heartbeat = Arc::clone(&mixer);
            jobs.spawn(async move {
                tokio::select! {
                    _ = mixer_shared.cancel.cancelled() => (),
                    _ = mixer_shared.listen() => (),
                    _ = mixer_heartbeat.heartbeat() => (),
                    _ = mixer_shared.history() => (),
                }
            });

            if mixer.meter.lock().await.has_history() {
                let mixer_shared = Arc::clone(&mixer);
                jobs.spawn(async move {
                    tokio::select! {
                        _ = mixer_shared.cancel.cancelled() => (),
                        _ = mixer_shared.check_timeout() => (),
                    }
                });
            }

            time::sleep(Duration::from_millis(50)).await;

            let mixer_shared = Arc::clone(&mixer);
            jobs.spawn(async move {
                tokio::select! {
                    _ = mixer_shared.cancel.cancelled() => (),
                    _ = mixer_shared.connect() => (),
                }
            });
        }

        mixer
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let mut jobs = self.jobs.lock().await;
        while let Some(_) = jobs.join_next().await {
        }
    }

    pub async fn history(&self) {
        let mut interval = time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let mut meter = self.meter.lock().await;
            meter.push_data();
        }
    }

    pub async fn reset_history(&self) {
        let mut history = self.meter.lock().await;
        history.reset();
    }

    pub async fn heartbeat(self: Arc<Self>) {
        let mut interval = time::interval(Duration::from_secs(8));
        loop {
            interval.tick().await;
            if send_osc(&self.socket, "/xremote", vec![])
                .await.log_err().is_err() { continue };
            if send_osc(
                    &self.socket,
                    "/meters",
                    vec![OscType::String("/meters/0".to_string()), OscType::Int(31)]
                ).await.log_err().is_err() {
                continue
            };

            let since_last_poll = {
                let meter = &*self.meter.lock().await;
                Instant::now() - meter.last_poll
            };

            let mut disconnection = false;
            {
                let mixer_state = &*self.state.lock().await;
                if since_last_poll > Duration::from_secs(10) {
                    match mixer_state {
                        MixerState::Disconnected => {},
                        _ => { disconnection = true; },
                    }
                } else {
                    match mixer_state {
                        MixerState::Disconnected => {
                            let mixer_shared = Arc::clone(&self);
                            let mut jobs = self.jobs.lock().await;
                            jobs.spawn(async move {
                                tokio::select! {
                                    _ = mixer_shared.cancel.cancelled() => (),
                                    _ = mixer_shared.connect() => (),
                                }
                            });
                        },
                        _ => {},
                    }
                }
            }

            if disconnection {
                log::warn!("Mixer disconnected!");
                let mut mixer_guard = self.state.lock().await;
                let _ = mem::replace(&mut *mixer_guard, MixerState::Disconnected);
            }
        }
    }

    pub async fn listen(&self) {
        let mut buf = [0u8; 4096];
        loop {
            let Ok(len) = self.socket.recv(&mut buf).await.log_err() else {
                continue;
            };
            if let Ok((_, OscPacket::Message(msg))) = rosc::decoder::decode_udp(&buf[..len]) {
                let addr = msg.addr.as_str();
                match addr {
                    "/meters/0" => {
                        if self.update_meter(msg.args).await.log_err().is_err() {
                            continue
                        };
                    },
                    addr => {
                        let Ok(arg) = parse_one_arg(msg.args).log_err() else {
                            continue;
                        };
                        let state = &mut *self.state.lock().await;
                        match state {
                            MixerState::Disconnected => {},
                            MixerState::Initializing(init) => {
                                if init.set_osc(addr, arg).log_err().is_err() {
                                    continue;
                                };
                            },
                            MixerState::Ready(ready) => {
                                if ready.update_osc(addr, arg).log_err().is_err() {
                                    continue;
                                };
                            },
                        };
                    }
                }
            }
        }
    }

    async fn check_timeout(&self) -> Result<()> {
        {
            let meter = self.meter.lock().await;
            if !meter.has_history() {
                anyhow::bail!("Timeout not set in config");
            }
        }
        loop {
            time::sleep(Duration::from_secs(60)).await;
            let mut meter = self.meter.lock().await;
            let mut timedout = false;
            if meter.history.len() == meter.max_history_size {
                let Some(max_meter) = meter.peak() else { continue };
                if max_meter < meter.config.as_ref().unwrap().db_threshold {
                    timedout = true;
                }
            }

            match meter.status {
                Timeout::TimedOut => {
                    if !timedout {
                        meter.status = Timeout::Running;
                    }
                },
                Timeout::Running => {
                    if timedout {
                        meter.status = Timeout::TimedOut;
                        let status = Command::new("sh")
                            .arg("-c")
                            .arg(meter.config.as_ref().unwrap().command.clone())
                            .status();
                        let Ok(status) = status.log_err() else {
                            continue;
                        };

                        if status.success() {
                            log::info!("Timeout command executed successfully")
                        } else {
                            log::warn!("Timeout command failed")
                        }
                    }
                }
            }
        }
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

    pub async fn connect(&self) {
        let mut next_sleep = Duration::from_secs(0);
        loop {
            tokio::time::sleep(next_sleep).await;
            let mut mutex_guard = self.state.lock().await;
            match &*mutex_guard {
                MixerState::Ready(_) => {
                    return
                },
                MixerState::Disconnected => {
                    let meter = self.meter.lock().await;
                    if Instant::now() - meter.last_poll < Duration::from_secs(5) {
                        log::info!("Mixer connected. Initialization in progress.");
                        let _ = mem::replace(&mut *mutex_guard, MixerState::Initializing(MixerStateInit::default()));
                        next_sleep = Duration::from_millis(100);
                    } else {
                        next_sleep = Duration::from_secs(10);
                    }
                },
                MixerState::Initializing(_) => {
                    let state = mem::replace(&mut *mutex_guard, MixerState::Disconnected);
                    if let MixerState::Initializing(mix_state) = state {
                        match mix_state.try_init(&self.socket, Duration::from_millis(25)).await {
                            Ok(ready) => {
                                log::info!("Mixer initialized.");
                                let _ = mem::replace(&mut *mutex_guard, MixerState::Ready(ready));
                                next_sleep = Duration::from_secs(0);
                            },
                            Err(init) => {
                                let _ = mem::replace(&mut *mutex_guard, MixerState::Initializing(init));
                                next_sleep = Duration::from_millis(200);
                            }
                        }
                    }
                }
            }
        }
    }
    
    pub async fn ready(&self) -> Option<MappedMutexGuard<'_,MixerStateReady>> {
        let mixer_guard = self.state.lock().await;
        MutexGuard::try_map(mixer_guard, |s| {
            match s {
                MixerState::Ready(ready) => Some(ready),
                _ => None
            }
        }).ok()
    }
}

