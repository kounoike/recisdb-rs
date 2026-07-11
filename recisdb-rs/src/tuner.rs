use crate::channels::Channel;
use std::str::FromStr;

#[cfg(target_os = "linux")]
pub use self::linux::{Tuner, UnTunedTuner};
#[cfg(target_os = "windows")]
pub use self::windows::{Tuner, UnTunedTuner};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

mod error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voltage {
    _11v,
    _15v,
    Low,
}

impl FromStr for Voltage {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "11v" | "_11v" => Ok(Self::_11v),
            "15v" | "_15v" => Ok(Self::_15v),
            "low" => Ok(Self::Low),
            _ => Err("expected one of: 11v, 15v, low"),
        }
    }
}

pub trait Tunable {
    fn tune(self, ch: Channel, lnb: Option<Voltage>) -> Result<Tuner, std::io::Error>;
}
