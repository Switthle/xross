use anyhow::{Context,Result};
use tokio::net::{UdpSocket};
use tokio::time::{sleep, Duration};
use rosc::{OscMessage, OscPacket, OscType};

#[derive(Debug)]
pub struct Field<const ADDR: &'static str, T: Copy> {
    pub val: T
}

#[derive(Debug,Default)]
pub struct ProtoField<const ADDR: &'static str, T: Copy> {
    pub val: Option<T>
}

impl<const ADDR: &'static str, T: Copy> ProtoField<ADDR,T> {
    pub fn try_to_field(self) -> Result<Field<ADDR,T>, &'static str> {
        Ok(
            Field {
                val: self.val.ok_or("Field is empty")?
            }
        )
    }



    pub fn set(&mut self, val: T) {
        self.val = Some(val)
    }
}


impl<const ADDR: &'static str> ProtoField<ADDR,f32> {
    pub fn set_osc(&mut self, val: OscType) -> Result<()> {
        let val = val.float().context("Wrong type")?;
        self.set(val);
        Ok(())
    }
}

impl<const ADDR: &'static str> ProtoField<ADDR,bool> {
    pub fn set_osc(&mut self, val: OscType) -> Result<()> {
        let val = val.int().context("Wrong type")?;
        self.set(val != 0);
        Ok(())
    }
}


impl<const ADDR: &'static str, T: Copy> Field<ADDR,T> {
    pub fn update(&mut self, val: T) {
        self.val = val
    }
}

impl<const ADDR: &'static str> Field<ADDR,f32> {
    pub fn update_osc(&mut self, val: OscType) -> Result<()> {
        let val = val.float().context("Wrong type")?;
        self.update(val);
        Ok(())
    }
}

impl<const ADDR: &'static str> Field<ADDR,bool> {
    pub fn update_osc(&mut self, val: OscType) -> Result<()> {
        let val = val.int().context("Wrong type")?;
        self.update(val != 0);
        Ok(())
    }
}



pub async fn send_osc(
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

pub async fn recv_osc(socket: &UdpSocket) -> Result<OscMessage> {
    let mut buf = [0u8; 4096];
    let len = socket.recv(&mut buf).await?;
    let packet = rosc::decoder::decode_udp(&buf[..len])
        .context("Failed to decode UDP packet")?;

    match packet {
        (_, OscPacket::Message(msg)) => Ok(msg),
        _ => anyhow::bail!("Not a message")
    }
}

pub fn parse_one_arg(args: Vec<OscType>) -> Result<OscType> {
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

pub async fn ask_and_wait(socket: &UdpSocket,addr: &str, wait: Option<Duration>) ->
Result<()> {
    send_osc(socket, addr, vec![]).await?;
    if let Some(wait) = wait {
        sleep(wait).await;
    }
    Ok(())
}
