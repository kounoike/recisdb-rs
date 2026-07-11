use std::future::Future;
use std::io;
use std::io::Write;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use b25_sys::DecoderOptions;
use futures_time::time::Duration;
use indicatif::ProgressBar;
use log::{info, warn};

use recisdb::channel::{Channel, ChannelType, TsFilter};
use recisdb::io::{StreamTransfer, TransferReport};
use recisdb::recording::{Recording, RecordingOptions};
use recisdb::tuner::{Tunable, UnTunedTuner};

use crate::commands::utils::{get_output, get_src, parse_keys};
use crate::context::{Cli, Commands};

pub mod utils;

pub type CommandFuture = Pin<Box<dyn Future<Output = io::Result<u64>>>>;

pub fn process_command(
    args: Cli,
) -> Result<(CommandFuture, Option<Duration>, Option<ProgressBar>), io::Error> {
    const INPUT_BUF_DEFAULT: usize = 200000;
    let buf_sz = std::env::var("RECISDB_INPUT_BUF_BYTES")
        .unwrap_or_default()
        .parse()
        .unwrap_or(INPUT_BUF_DEFAULT);

    Ok(match args.command {
        Commands::Checksignal {
            channel,
            device,
            lnb,
        } => {
            let channel = channel.map(|ch| Channel::new(ch, None)).unwrap();
            if let ChannelType::BS(_, TsFilter::RelTsNum(num)) = channel.ch_type {
                warn!("The specified relative TS num '_{}' has no effect.", num)
            }
            if let ChannelType::Undefined = channel.ch_type {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The specified channel is invalid.",
                ));
            }
            info!("Tuner: {}", device);
            info!(
                "Channel: {} / {}",
                channel.get_raw_ch_name(),
                channel.ch_type
            );

            let tuned = UnTunedTuner::new(device, 0)?.tune(channel, lnb)?;
            let stop_requested = Arc::new(AtomicBool::new(false));
            let stop_handle = Arc::clone(&stop_requested);
            ctrlc::set_handler(move || stop_handle.store(true, Ordering::Relaxed))
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            let fut: CommandFuture = Box::pin(async move {
                while !stop_requested.load(Ordering::Relaxed) {
                    print!("\r{:.2}dB", tuned.signal_quality());
                    std::io::stdout().flush()?;
                    std::thread::sleep(std::time::Duration::from_secs_f64(1.0));
                }
                Ok(0)
            });
            (fut, None, None)
        }
        Commands::Tune {
            device,
            channel,
            card,
            tsid,
            time,
            no_decode: disable_decode,
            lnb,
            key0,
            key1,
            no_simd,
            no_strip,
            output,
            exit_on_card_error,
        } => {
            if let Some(name) = card {
                #[cfg(not(feature = "prioritized_card_reader"))]
                warn!("--card {name} has no effect. Use `prioritized_card_reader` feature flag.");

                #[cfg(feature = "prioritized_card_reader")]
                b25_sys::set_card_reader_name(&name);
            }

            let channel = channel.map(|ch| Channel::new(ch, tsid)).unwrap();
            if let ChannelType::Undefined = channel.ch_type {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The specified channel is invalid.",
                ));
            }
            info!("Tuner: {}", device);
            info!(
                "Channel: {} / {}",
                channel.get_raw_ch_name(),
                channel.ch_type
            );

            let rec_duration = time.map(Duration::from_secs_f64);
            match rec_duration {
                Some(duration) => info!("Recording duration: {} seconds", duration.as_secs_f64()),
                None => info!("Recording duration: Infinite"),
            }

            let decoder = if disable_decode {
                info!("Decode: Disabled");
                None
            } else {
                info!("Decode: Enabled");
                Some(DecoderOptions {
                    enable_working_key: parse_keys(key0, key1),
                    simd: !no_simd,
                    strip: !no_strip,
                    ..DecoderOptions::default()
                })
            };

            let output = get_output(output)?;
            let recording_options = RecordingOptions {
                device,
                channel,
                lnb,
                tsid,
                decoder,
                continue_on_decoder_error: !exit_on_card_error,
                input_buffer_size: buf_sz,
            };
            let (handle, recording) = Recording::start(recording_options, output)
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;

            let stop_handle = handle.clone();
            ctrlc::set_handler(move || stop_handle.stop())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            let fut: CommandFuture = Box::pin(async move {
                recording
                    .await
                    .map(|report| report.written_bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
            });
            (fut, rec_duration, None)
        }
        Commands::Decode {
            source,
            card,
            key0,
            key1,
            no_simd,
            no_strip,
            output,
        } => {
            if let Some(name) = card {
                #[cfg(not(feature = "prioritized_card_reader"))]
                warn!("--card {name} has no effect. Use `prioritized_card_reader` feature flag.");

                #[cfg(feature = "prioritized_card_reader")]
                b25_sys::set_card_reader_name(&name);
            }

            let (input, input_sz) = get_src(None, None, source, None, buf_sz)?;
            let output = get_output(output)?;
            let decoder = Some(DecoderOptions {
                enable_working_key: parse_keys(key0, key1),
                simd: !no_simd,
                strip: !no_strip,
                ..DecoderOptions::default()
            });

            let stop_requested = Arc::new(AtomicBool::new(false));
            let stop_handle = Arc::clone(&stop_requested);
            ctrlc::set_handler(move || stop_handle.store(true, Ordering::Relaxed))
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            let transfer = StreamTransfer::new(input, output, decoder, false, stop_requested)?;
            let (transfer, pb) = if let Some(size) = input_sz {
                let pb = crate::utils::init_progress(size);
                let cb = pb.clone();
                (
                    transfer.with_progress_callback(move |pos| cb.set_position(pos)),
                    Some(pb),
                )
            } else {
                (transfer, None)
            };

            let fut: CommandFuture = Box::pin(async move {
                transfer
                    .await
                    .map(|TransferReport { written_bytes, .. }| written_bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
            });
            (fut, None, pb)
        }
        #[cfg(windows)]
        Commands::Enumerate { device, space } => {
            let untuned = UnTunedTuner::new(device, buf_sz)?;
            let fut: CommandFuture = Box::pin(async move {
                if let Some(spacename_channels) = untuned.enum_channels(space) {
                    for item in spacename_channels {
                        println!("{}", item)
                    }
                    Ok(0)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Failed to enumerate channels.",
                    ))
                }
            });
            (fut, None, None)
        }
    })
}
