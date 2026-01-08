use std::path::PathBuf;

use tracing::warn;

pub const REST_PORT: u16 = 7891;
pub const GRPC_PORT: u16 = 7892;
pub const SYNC_HOUR_UTC: u8 = 2;
pub const CSV_URL: &str =
    "https://github.com/NetworkCats/OpenProxyDB/releases/latest/download/proxy_blocks.csv";

#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub rest_port: u16,
    pub grpc_port: u16,
    pub sync_hour_utc: u8,
    pub csv_url: String,
}

fn parse_port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|s| {
            let port: u16 = s.parse().ok()?;
            if port == 0 {
                warn!("{} cannot be 0, using default {}", var, default);
                None
            } else {
                Some(port)
            }
        })
        .unwrap_or(default)
}

fn parse_sync_hour(default: u8) -> u8 {
    std::env::var("PROXYD_SYNC_HOUR_UTC")
        .ok()
        .and_then(|s| {
            let hour: u8 = s.parse().ok()?;
            if hour > 23 {
                warn!(
                    "PROXYD_SYNC_HOUR_UTC must be 0-23, got {}, using default {}",
                    hour, default
                );
                None
            } else {
                Some(hour)
            }
        })
        .unwrap_or(default)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(
                std::env::var("PROXYD_DATA_DIR").unwrap_or_else(|_| "/data".to_string()),
            ),
            rest_port: parse_port("PROXYD_REST_PORT", REST_PORT),
            grpc_port: parse_port("PROXYD_GRPC_PORT", GRPC_PORT),
            sync_hour_utc: parse_sync_hour(SYNC_HOUR_UTC),
            csv_url: std::env::var("PROXYD_CSV_URL").unwrap_or_else(|_| CSV_URL.to_string()),
        }
    }
}

impl Config {
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("lmdb")
    }

    pub fn csv_path(&self) -> PathBuf {
        self.data_dir.join("proxy_blocks.csv")
    }

    pub fn csv_hash_path(&self) -> PathBuf {
        self.data_dir.join("proxy_blocks.csv.sha256")
    }
}
