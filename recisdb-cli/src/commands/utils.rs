use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;

use futures_util::io::{AllowStdIo, BufReader};
use futures_util::AsyncBufRead;

use recisdb::channel::Channel;
use recisdb::tuner::{Tunable, UnTunedTuner, Voltage};

pub fn get_src(
    device: Option<String>,
    channel: Option<Channel>,
    source: Option<String>,
    lnb: Option<Voltage>,
    buf_sz: usize,
) -> Result<(Box<dyn AsyncBufRead + Unpin>, Option<u64>), io::Error> {
    match (device, channel, source) {
        (Some(device), Some(channel), None) => {
            let inner = UnTunedTuner::new(device, buf_sz)?.tune(channel, lnb)?;
            Ok((Box::new(inner) as Box<dyn AsyncBufRead + Unpin>, None))
        }
        (None, None, Some(src)) => {
            if src == "-" {
                let input = BufReader::with_capacity(8192, AllowStdIo::new(io::stdin().lock()));
                return Ok((Box::new(input) as Box<dyn AsyncBufRead + Unpin>, None));
            }

            let src = fs::canonicalize(src)?;
            let src_sz = fs::metadata(&src)
                .ok()
                .and_then(|m| m.is_file().then_some(m.len()));
            let input = BufReader::with_capacity(20000, AllowStdIo::new(fs::File::open(src)?));
            Ok((Box::new(input) as Box<dyn AsyncBufRead + Unpin>, src_sz))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Either device & channel or source must be specified.",
        )),
    }
}

pub fn get_output(path: Option<String>) -> Result<Box<dyn Write>, io::Error> {
    match path {
        Some(s) if s == "-" => Ok(Box::new(std::io::stdout().lock()) as Box<dyn Write>),
        Some(s) if s == "/dev/null" => Ok(Box::new(fs::File::create(s)?)),
        Some(path) => {
            let p = Path::new(&path);
            let path_buf;
            if p.exists() {
                if p.is_file() {
                    return Ok(Box::new(fs::File::create(p)?));
                } else {
                    path_buf = p.to_path_buf();
                }
            } else if path.ends_with('/') || (cfg!(windows) && path.ends_with('\\')) {
                fs::create_dir_all(&path)?;
                path_buf = p.to_path_buf();
            } else {
                let parent = p
                    .parent()
                    .ok_or(io::Error::new(io::ErrorKind::Other, "Invalid path"))?;
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
                return Ok(Box::new(fs::File::create(p)?));
            }
            let filename_time_now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            Ok(Box::new(fs::File::create(format!(
                "{}/{}.m2ts",
                path_buf.to_str().unwrap(),
                filename_time_now
            ))?))
        }
        None => Err(io::Error::new(
            io::ErrorKind::Other,
            "No output path specified.",
        )),
    }
}

pub fn parse_keys(key0: Option<Vec<String>>, key1: Option<Vec<String>>) -> bool {
    match (key0, key1) {
        (None, None) => false,
        (Some(k0), Some(k1)) => {
            let k0 = k0
                .iter()
                .map(|k| u64::from_str_radix(k.trim_start_matches("0x"), 16).unwrap())
                .collect::<Vec<u64>>();
            let k1 = k1
                .iter()
                .map(|k| u64::from_str_radix(k.trim_start_matches("0x"), 16).unwrap())
                .collect::<Vec<u64>>();
            b25_sys::set_keys(k0, k1);
            true
        }
        _ => panic!("Specify both of the keys."),
    }
}
