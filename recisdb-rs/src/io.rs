use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::io::Write;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use futures_util::io::{AllowStdIo, BufReader};
use futures_util::{AsyncBufRead, AsyncWrite};
use log::{info, warn};
use pin_project_lite::pin_project;

use b25_sys::{DecoderOptions, StreamDecoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStopReason {
    Completed,
    InputEof,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferReport {
    pub written_bytes: u64,
    pub stop_reason: TransferStopReason,
}

type ProgressCallback = Arc<dyn Fn(u64) + Send + Sync + 'static>;

pin_project! {
    pub struct StreamTransfer {
        #[pin]
        input: Box<dyn AsyncBufRead + Unpin + 'static>,
        output: AllowStdIo<Box<dyn Write>>,
        decoder: RefCell<Option<BufReader<AllowStdIo<StreamDecoder>>>>,
        written: u64,
        finalize_reason: Option<TransferStopReason>,
        stop_requested: Arc<AtomicBool>,
        decoder_fallback: bool,
        decoder_disabled: bool,
        progress_callback: Option<ProgressCallback>,
        last_progress: u64,
    }
}

impl StreamTransfer {
    const DECODER_BUFFER_CAPACITY: usize = 1_600_000;

    pub fn new(
        input: Box<dyn AsyncBufRead + Unpin>,
        output: Box<dyn Write>,
        decoder_options: Option<DecoderOptions>,
        continue_on_decoder_error: bool,
        stop_requested: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let decoder = match decoder_options {
            Some(options) => match StreamDecoder::new(options) {
                Ok(raw) => Some(BufReader::with_capacity(
                    Self::DECODER_BUFFER_CAPACITY,
                    AllowStdIo::new(raw),
                )),
                Err(error) if continue_on_decoder_error => {
                    warn!("Failed to initialize the decoder ({error}). Falling back to pass-through mode.");
                    None
                }
                Err(error) => {
                    return Err(io::Error::new(io::ErrorKind::Other, error));
                }
            },
            None => None,
        };

        Ok(Self {
            input,
            output: AllowStdIo::new(output),
            decoder: RefCell::new(decoder),
            written: 0,
            finalize_reason: None,
            stop_requested,
            decoder_fallback: continue_on_decoder_error,
            decoder_disabled: false,
            progress_callback: None,
            last_progress: 0,
        })
    }

    pub fn with_progress_callback(
        mut self,
        callback: impl Fn(u64) + Send + Sync + 'static,
    ) -> Self {
        self.progress_callback = Some(Arc::new(callback));
        self
    }
}

impl Future for StreamTransfer {
    type Output = io::Result<TransferReport>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        let stop_requested = this.stop_requested.load(Ordering::Relaxed);
        if stop_requested {
            *this.finalize_reason = Some(TransferStopReason::Stopped);
        }

        let mut finalize_reason = *this.finalize_reason;
        if !*this.decoder_disabled {
            let mut decoder_guard = this.decoder.borrow_mut();
            if let Some(decoder) = decoder_guard.as_mut() {
                if finalize_reason.is_none() {
                    let buffer = ready!(this.input.as_mut().poll_fill_buf(cx))?;
                    if buffer.is_empty() {
                        finalize_reason = Some(TransferStopReason::InputEof);
                        *this.finalize_reason = finalize_reason;
                    } else {
                        match ready!(Pin::new(decoder).poll_write(cx, buffer)) {
                            Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                            Ok(written) => {
                                *this.written += written as u64;
                                this.input.as_mut().consume(written);
                                if let Some(callback) = this.progress_callback.as_ref() {
                                    if *this.last_progress != *this.written {
                                        *this.last_progress = *this.written;
                                        callback(*this.written);
                                    }
                                }
                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            }
                            Err(error) => {
                                if *this.decoder_fallback {
                                    warn!(
                                        "Unexpected decoder failure ({error}). Falling back to pass-through mode."
                                    );
                                    *this.decoder_disabled = true;
                                    cx.waker().wake_by_ref();
                                    return Poll::Pending;
                                } else {
                                    return Poll::Ready(Err(error));
                                }
                            }
                        }
                    }
                }

                info!("Flushing the decoder buffer...");
                ready!(Pin::new(&mut *decoder).poll_flush(cx))?;

                loop {
                    match Pin::new(&mut *decoder).poll_fill_buf(cx) {
                        Poll::Ready(Ok(buffer)) if buffer.is_empty() => {
                            ready!(Pin::new(&mut this.output).poll_flush(cx))?;
                            let report = TransferReport {
                                written_bytes: *this.written,
                                stop_reason: finalize_reason
                                    .unwrap_or(TransferStopReason::Completed),
                            };
                            return Poll::Ready(Ok(report));
                        }
                        Poll::Ready(Ok(buffer)) => {
                            let bytes = ready!(Pin::new(&mut this.output).poll_write(cx, buffer))?;
                            if bytes == 0 {
                                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
                            }
                            Pin::new(&mut *decoder).consume(bytes);
                            *this.written += bytes as u64;
                            if let Some(callback) = this.progress_callback.as_ref() {
                                if *this.last_progress != *this.written {
                                    *this.last_progress = *this.written;
                                    callback(*this.written);
                                }
                            }
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Pending => continue,
                    }
                }
            }
        }

        let buffer = ready!(this.input.as_mut().poll_fill_buf(cx))?;
        if buffer.is_empty() {
            ready!(Pin::new(&mut this.output).poll_flush(cx))?;
            let report = TransferReport {
                written_bytes: *this.written,
                stop_reason: finalize_reason.unwrap_or(TransferStopReason::InputEof),
            };
            return Poll::Ready(Ok(report));
        }

        let bytes = ready!(Pin::new(&mut this.output).poll_write(cx, buffer))?;
        if bytes == 0 {
            return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
        }
        *this.written += bytes as u64;
        this.input.as_mut().consume(bytes);
        if let Some(callback) = this.progress_callback.as_ref() {
            if *this.last_progress != *this.written {
                *this.last_progress = *this.written;
                callback(*this.written);
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
