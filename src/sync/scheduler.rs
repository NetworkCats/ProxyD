use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, Utc};
use thiserror::Error;
use tokio::time::{sleep, Duration as TokioDuration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::Config;
use crate::db::{Database, DbError, Metadata};
use crate::metrics;
use crate::sync::downloader::{download_csv, load_hash, DownloadError};
use crate::sync::importer::{full_import, incremental_import, ImportError};

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("Download failed: {0}")]
    Download(#[from] DownloadError),
    #[error("Import failed: {0}")]
    Import(#[from] ImportError),
    #[error("Database error: {0}")]
    Database(#[from] DbError),
}

fn duration_until_next_sync(target_hour: u8) -> TokioDuration {
    let now = Utc::now();
    let target_hour = u32::from(target_hour);

    let today_target = now
        .date_naive()
        .and_hms_opt(target_hour, 0, 0)
        .expect("valid time");
    let today_target = today_target.and_utc();

    let next_sync = if now.naive_utc() < today_target.naive_utc() {
        today_target
    } else {
        today_target + Duration::days(1)
    };

    let duration_secs = (next_sync - now).num_seconds().max(0) as u64;
    TokioDuration::from_secs(duration_secs)
}

fn update_metrics_from_db(meta: &Metadata) {
    #[allow(clippy::cast_possible_wrap)]
    metrics::set_record_count(meta.record_count as i64);
    if let Some(ts) = meta.last_sync {
        metrics::set_last_sync_timestamp(ts);
    }
}

pub async fn run_scheduler(db: Arc<Database>, config: Config, cancel_token: CancellationToken) {
    loop {
        let sleep_duration = duration_until_next_sync(config.sync_hour_utc);
        info!(
            "Next sync scheduled in {} hours {} minutes",
            sleep_duration.as_secs() / 3600,
            (sleep_duration.as_secs() % 3600) / 60
        );

        tokio::select! {
            () = sleep(sleep_duration) => {
                info!("Starting scheduled sync at {} UTC", config.sync_hour_utc);
                let start = Instant::now();
                if let Err(e) = perform_sync(&db, &config).await {
                    error!("Sync failed: {}", e);
                    metrics::inc_sync_failures();
                } else {
                    metrics::inc_sync_success();
                }
                metrics::record_sync_duration(start.elapsed().as_secs_f64());
            }
            () = cancel_token.cancelled() => {
                info!("Scheduler received shutdown signal");
                break;
            }
        }
    }
}

pub async fn perform_sync(db: &Arc<Database>, config: &Config) -> Result<(), SyncError> {
    info!("Starting scheduled sync");

    let result = download_csv(&config.csv_url).await?;

    let current_hash = load_hash(&config.csv_hash_path()).await;
    let is_first_run = db.is_empty()?;

    if is_first_run {
        full_import(db, &result.content, &result.hash, config).await?;
    } else if current_hash.as_ref() != Some(&result.hash) {
        incremental_import(db, &result.content, &result.hash, config).await?;
    } else {
        info!("CSV unchanged, skipping import");
    }

    if let Ok(meta) = db.get_metadata() {
        update_metrics_from_db(&meta);
    }

    Ok(())
}

pub async fn initial_sync(db: &Arc<Database>, config: &Config) -> Result<(), SyncError> {
    info!("Performing initial sync");

    let is_empty = db.is_empty()?;

    if is_empty {
        if config.csv_path().exists() {
            info!("Database empty but local CSV exists, rebuilding from CSV");
            crate::sync::rebuild_from_csv(db, config).await?;
        } else {
            info!("First run, downloading CSV");
            let result = download_csv(&config.csv_url).await?;
            full_import(db, &result.content, &result.hash, config).await?;
        }
    } else {
        info!("Database already populated, skipping initial sync");
    }

    if let Ok(meta) = db.get_metadata() {
        update_metrics_from_db(&meta);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    #[test]
    fn test_duration_until_next_sync_returns_valid_duration() {
        let duration = duration_until_next_sync(3);
        assert!(duration.as_secs() <= 24 * 60 * 60);
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn test_duration_until_next_sync_same_hour_schedules_tomorrow() {
        let current_hour = Utc::now().hour() as u8;
        let duration = duration_until_next_sync(current_hour);
        // Should be close to 24 hours (minus a few seconds that elapsed)
        assert!(duration.as_secs() >= 23 * 60 * 60);
    }
}
