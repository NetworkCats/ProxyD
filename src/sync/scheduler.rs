use std::sync::Arc;
use std::time::Instant;

use chrono::{Timelike, Utc};
use tokio::time::{sleep, Duration as TokioDuration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::Config;
use crate::db::Database;
use crate::metrics;
use crate::sync::downloader::{download_csv, load_hash};
use crate::sync::importer::{full_import, incremental_import};

const CHECK_INTERVAL_SECS: u64 = 60;

fn should_sync_now(target_hour: u8, last_sync_date: &mut Option<chrono::NaiveDate>) -> bool {
    let now = Utc::now();
    let today = now.date_naive();

    if now.hour() == u32::from(target_hour) && *last_sync_date != Some(today) {
        *last_sync_date = Some(today);
        return true;
    }
    false
}

pub async fn run_scheduler(db: Arc<Database>, config: Config, cancel_token: CancellationToken) {
    let mut last_sync_date: Option<chrono::NaiveDate> = None;

    loop {
        if should_sync_now(config.sync_hour_utc, &mut last_sync_date) {
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

        tokio::select! {
            () = sleep(TokioDuration::from_secs(CHECK_INTERVAL_SECS)) => {}
            () = cancel_token.cancelled() => {
                info!("Scheduler received shutdown signal");
                break;
            }
        }
    }
}

pub async fn perform_sync(db: &Arc<Database>, config: &Config) -> Result<(), String> {
    info!("Starting scheduled sync");

    let result = download_csv(&config.csv_url)
        .await
        .map_err(|e| e.to_string())?;

    let current_hash = load_hash(&config.csv_hash_path()).await;
    let is_first_run = db.is_empty().unwrap_or(true);

    if is_first_run {
        full_import(db, &result.content, &result.hash, config)
            .await
            .map_err(|e| e.to_string())?;
    } else if current_hash.as_ref() != Some(&result.hash) {
        incremental_import(db, &result.content, &result.hash, config)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        info!("CSV unchanged, skipping import");
    }

    if let Ok(meta) = db.get_metadata() {
        #[allow(clippy::cast_possible_wrap)]
        metrics::set_record_count(meta.record_count as i64);
        if let Some(ts) = meta.last_sync {
            metrics::set_last_sync_timestamp(ts);
        }
    }

    Ok(())
}

pub async fn initial_sync(db: &Arc<Database>, config: &Config) -> Result<(), String> {
    info!("Performing initial sync");

    let is_empty = db.is_empty().unwrap_or(true);

    if is_empty {
        if config.csv_path().exists() {
            info!("Database empty but local CSV exists, rebuilding from CSV");
            crate::sync::rebuild_from_csv(db, config)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            info!("First run, downloading CSV");
            let result = download_csv(&config.csv_url)
                .await
                .map_err(|e| e.to_string())?;
            full_import(db, &result.content, &result.hash, config)
                .await
                .map_err(|e| e.to_string())?;
        }
    } else {
        info!("Database already populated, skipping initial sync");
    }

    if let Ok(meta) = db.get_metadata() {
        #[allow(clippy::cast_possible_wrap)]
        metrics::set_record_count(meta.record_count as i64);
        if let Some(ts) = meta.last_sync {
            metrics::set_last_sync_timestamp(ts);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn test_should_sync_now() {
        let mut last_sync_date = None;
        let current_hour = Utc::now().hour() as u8;

        let should_sync = should_sync_now(current_hour, &mut last_sync_date);
        assert!(should_sync);
        assert!(last_sync_date.is_some());

        let should_sync_again = should_sync_now(current_hour, &mut last_sync_date);
        assert!(!should_sync_again);
    }
}
