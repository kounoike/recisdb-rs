use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use b25_sys::DecoderOptions;
use futures_util::AsyncBufRead;

use crate::channels::{Channel, ChannelType};
use crate::io::{StreamTransfer, TransferReport, TransferStopReason};
use crate::tuner::{Tunable, UnTunedTuner, Voltage};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidChannel(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::InvalidChannel(value) => write!(f, "Invalid channel: {value}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct RecordingOptions {
    pub device: String,
    pub channel: Channel,
    pub lnb: Option<Voltage>,
    pub tsid: Option<u32>,
    pub decoder: Option<DecoderOptions>,
    pub continue_on_decoder_error: bool,
    pub input_buffer_size: usize,
}

impl RecordingOptions {
    pub fn new(device: impl Into<String>, channel: Channel) -> Self {
        Self {
            device: device.into(),
            channel,
            lnb: None,
            tsid: None,
            decoder: None,
            continue_on_decoder_error: false,
            input_buffer_size: 200_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStopReason {
    Completed,
    InputEof,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingReport {
    pub written_bytes: u64,
    pub stop_reason: RecordingStopReason,
}

#[derive(Clone)]
pub struct RecordingHandle {
    stop_requested: Arc<AtomicBool>,
}

impl RecordingHandle {
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::Relaxed);
    }
}

pub struct Recording {
    inner: StreamTransfer,
}

impl Recording {
    pub fn start(
        options: RecordingOptions,
        output: Box<dyn std::io::Write>,
    ) -> Result<(RecordingHandle, Self), Error> {
        if let ChannelType::Undefined = options.channel.ch_type {
            return Err(Error::InvalidChannel(
                options.channel.get_raw_ch_name().to_string(),
            ));
        }

        let stop_requested = Arc::new(AtomicBool::new(false));
        let handle = RecordingHandle {
            stop_requested: Arc::clone(&stop_requested),
        };

        let tuned = UnTunedTuner::new(options.device, options.input_buffer_size)?
            .tune(options.channel, options.lnb)?;
        let input: Box<dyn AsyncBufRead + Unpin> = Box::new(tuned);
        let inner = StreamTransfer::new(
            input,
            output,
            options.decoder,
            options.continue_on_decoder_error,
            stop_requested,
        )?;

        Ok((handle, Self { inner }))
    }

    pub fn with_progress_callback(
        mut self,
        callback: impl Fn(u64) + Send + Sync + 'static,
    ) -> Self {
        self.inner = self.inner.with_progress_callback(callback);
        self
    }
}

impl Future for Recording {
    type Output = Result<RecordingReport, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll(cx) {
            Poll::Ready(Ok(TransferReport {
                written_bytes,
                stop_reason,
            })) => {
                let reason = match stop_reason {
                    TransferStopReason::Completed => RecordingStopReason::Completed,
                    TransferStopReason::InputEof => RecordingStopReason::InputEof,
                    TransferStopReason::Stopped => RecordingStopReason::Stopped,
                };
                Poll::Ready(Ok(RecordingReport {
                    written_bytes,
                    stop_reason: reason,
                }))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(Error::Io(error))),
            Poll::Pending => Poll::Pending,
        }
    }
}
