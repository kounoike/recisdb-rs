use clap::Parser;
use futures_executor::block_on;
use futures_time::future::FutureExt;
use log::error;

mod commands;
mod context;
mod utils;

fn main() {
    let arg = context::Cli::parse();
    utils::initialize_logger();

    let (fut, timeout_option, progress) = match commands::process_command(arg) {
        Ok(value) => value,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    let result = match timeout_option {
        Some(dur) => match block_on(fut.timeout(dur)) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Timed out",
            )),
        },
        None => block_on(fut).map(|_| ()),
    };

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    if let Err(error) = result {
        if error.kind() != std::io::ErrorKind::TimedOut {
            error!("{error}");
            std::process::exit(1);
        }
    }
}
