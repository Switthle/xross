use std::fmt::Display;
use log;

pub trait LogEntryExt {
    fn log_err(self) -> Self;
}

impl<T, E> LogEntryExt for Result<T, E> 
where 
    E: Display 
{
    fn log_err(self) -> Self {
        if let Err(ref e) = self {
            log::warn!("{:#}", e);
        }
        self
    }
}
