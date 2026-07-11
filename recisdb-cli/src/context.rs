use clap::{ArgGroup, Parser, Subcommand};
use clap_num::maybe_hex;

use recisdb::Voltage;

fn parse_voltage(value: &str) -> Result<Voltage, String> {
    value.parse::<Voltage>().map_err(|e| e.to_string())
}

#[derive(Debug, Parser)]
#[clap(name = "recisdb")]
#[clap(about = "recisdb can read both Unix chardev-based and BonDriver-based TV sources. ", long_about = None)]
#[clap(author = "maleicacid")]
#[clap(version)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[clap(name = "checksignal")]
    Checksignal {
        #[clap(short, long, value_name = "CANONICAL_PATH", required = true)]
        device: String,
        #[clap(short, long, required = true)]
        channel: Option<String>,
        #[clap(value_parser = parse_voltage, long = "lnb")]
        lnb: Option<Voltage>,
    },
    #[clap(group(
    ArgGroup::new("key")
    .args(& ["key0", "key1"])
    .requires_all(& ["key0", "key1"])
    .multiple(true)
    ))]
    Tune {
        #[clap(short = 'i', long, value_name = "CANONICAL_PATH", required = true)]
        device: String,
        #[clap(short, long, required = true)]
        channel: Option<String>,
        #[clap(long)]
        card: Option<String>,
        #[clap(long, value_parser=maybe_hex::<u32>)]
        tsid: Option<u32>,
        #[clap(short, long, value_name = "seconds")]
        time: Option<f64>,
        #[clap(short = 'e', long)]
        exit_on_card_error: bool,
        #[clap(long = "no-decode")]
        no_decode: bool,
        #[clap(long = "no-simd")]
        no_simd: bool,
        #[clap(long = "no-strip")]
        no_strip: bool,
        #[clap(value_parser = parse_voltage, long = "lnb")]
        lnb: Option<Voltage>,
        #[clap(long = "key0")]
        key0: Option<Vec<String>>,
        #[clap(long = "key1")]
        key1: Option<Vec<String>>,
        #[clap(required = true)]
        output: Option<String>,
    },
    #[clap(group(
    ArgGroup::new("key")
    .args(& ["key0", "key1"])
    .requires_all(& ["key0", "key1"])
    .multiple(true)
    ))]
    Decode {
        #[clap(short = 'i', long = "input", value_name = "file", required = true)]
        source: Option<String>,
        #[clap(long = "no-simd")]
        no_simd: bool,
        #[clap(long = "no-strip")]
        no_strip: bool,
        #[clap(long)]
        card: Option<String>,
        #[clap(long = "key0")]
        key0: Option<Vec<String>>,
        #[clap(long = "key1")]
        key1: Option<Vec<String>>,
        #[clap(required = true)]
        output: Option<String>,
    },
    #[cfg(windows)]
    Enumerate {
        #[clap(short = 'i', long, value_name = "CANONICAL_PATH", required = true)]
        device: String,
        #[clap(short, long, required = true)]
        space: u32,
    },
}
