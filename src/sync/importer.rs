use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use rayon::prelude::*;
use thiserror::Error;
use tracing::info;

use crate::config::Config;
use crate::db::{Database, DbError, Metadata};
use crate::ip::{IpTrie, ReputationFlags};
use crate::sync::downloader::{compute_hash, load_csv, load_hash, save_csv, save_hash};

#[derive(Error, Debug)]
pub enum ImportError {
    #[error("CSV parse error: {0}")]
    CsvParse(String),
    #[error("Database error: {0}")]
    Database(#[from] DbError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Download error: {0}")]
    Download(#[from] crate::sync::downloader::DownloadError),
    #[error("LMDB error: {0}")]
    Heed(#[from] heed::Error),
}

#[derive(Debug, Clone)]
pub struct CsvRecord {
    pub ip: String,
    pub flags: ReputationFlags,
}

fn parse_bool(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes")
}

pub fn parse_csv_parallel(content: &str) -> Result<Vec<CsvRecord>, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| ImportError::CsvParse(e.to_string()))?
        .clone();

    let header_indices = HeaderIndices::from_headers(&headers);

    let raw_records: Vec<csv::StringRecord> = reader.records().filter_map(Result::ok).collect();

    let records: Vec<CsvRecord> = raw_records
        .par_iter()
        .filter_map(|record| {
            let ip = record.get(0)?.to_owned();
            if ip.is_empty() {
                return None;
            }

            let flags = header_indices.extract_flags(record);
            Some(CsvRecord { ip, flags })
        })
        .collect();

    Ok(records)
}

struct HeaderIndices {
    anonblock: Option<usize>,
    proxy: Option<usize>,
    vpn: Option<usize>,
    cdn: Option<usize>,
    public_wifi: Option<usize>,
    rangeblock: Option<usize>,
    school_block: Option<usize>,
    tor: Option<usize>,
    webhost: Option<usize>,
}

impl HeaderIndices {
    fn from_headers(headers: &csv::StringRecord) -> Self {
        let find_index = |name: &str| headers.iter().position(|h| h == name);

        Self {
            anonblock: find_index("anonblock"),
            proxy: find_index("proxy"),
            vpn: find_index("vpn"),
            cdn: find_index("cdn"),
            public_wifi: find_index("public-wifi"),
            rangeblock: find_index("rangeblock"),
            school_block: find_index("school-block"),
            tor: find_index("tor"),
            webhost: find_index("webhost"),
        }
    }

    fn extract_flags(&self, record: &csv::StringRecord) -> ReputationFlags {
        #[allow(clippy::map_unwrap_or)]
        let get_bool = |idx: Option<usize>| -> bool {
            idx.and_then(|i| record.get(i))
                .map(parse_bool)
                .unwrap_or(false)
        };

        ReputationFlags {
            anonblock: get_bool(self.anonblock),
            proxy: get_bool(self.proxy),
            vpn: get_bool(self.vpn),
            cdn: get_bool(self.cdn),
            public_wifi: get_bool(self.public_wifi),
            rangeblock: get_bool(self.rangeblock),
            school_block: get_bool(self.school_block),
            tor: get_bool(self.tor),
            webhost: get_bool(self.webhost),
        }
    }
}

const BATCH_COMMIT_SIZE: usize = 10_000;

fn do_full_import(
    db: &Arc<Database>,
    records: &[CsvRecord],
    hash: &str,
) -> Result<u64, ImportError> {
    let count = records.len() as u64;

    {
        let mut txn = db.begin_write()?;
        db.clear_all(&mut txn)?;
        txn.commit()?;
    }

    let mut trie = IpTrie::new();
    let mut batch_count = 0;
    let mut txn = db.begin_write()?;

    for record in records {
        db.insert_record(&mut txn, &record.ip, &record.flags)?;

        if let Ok(network) = record.ip.parse() {
            trie.insert(network, record.flags);
        }

        batch_count += 1;
        if batch_count >= BATCH_COMMIT_SIZE {
            txn.commit()?;
            txn = db.begin_write()?;
            batch_count = 0;
        }
    }

    let metadata = Metadata {
        last_sync: Some(Utc::now().timestamp()),
        csv_hash: Some(hash.to_owned()),
        record_count: count,
    };
    db.set_metadata(&mut txn, &metadata)?;
    txn.commit()?;

    db.swap_trie(trie);

    Ok(count)
}

fn do_incremental_import(
    db: &Arc<Database>,
    new_records: &[CsvRecord],
    hash: &str,
) -> Result<(u64, u64, u64), ImportError> {
    let existing = db.get_all_entries()?;
    let existing_map: HashMap<&str, &ReputationFlags> =
        existing.iter().map(|(k, f)| (k.as_str(), f)).collect();

    let new_keys: HashSet<&str> = new_records.iter().map(|r| r.ip.as_str()).collect();

    let mut added = 0u64;
    let mut updated = 0u64;
    let mut deleted = 0u64;

    let mut txn = db.begin_write()?;

    for record in new_records {
        match existing_map.get(record.ip.as_str()) {
            None => {
                db.insert_record(&mut txn, &record.ip, &record.flags)?;
                added += 1;
            }
            Some(existing_flags) if *existing_flags != &record.flags => {
                db.insert_record(&mut txn, &record.ip, &record.flags)?;
                updated += 1;
            }
            Some(_) => {}
        }
    }

    for (ip, _) in &existing {
        if !new_keys.contains(ip.as_str()) {
            db.delete_record(&mut txn, ip)?;
            deleted += 1;
        }
    }

    let metadata = Metadata {
        last_sync: Some(Utc::now().timestamp()),
        csv_hash: Some(hash.to_owned()),
        record_count: new_records.len() as u64,
    };
    db.set_metadata(&mut txn, &metadata)?;

    txn.commit()?;
    db.rebuild_trie()?;

    Ok((added, updated, deleted))
}

pub async fn full_import(
    db: &Arc<Database>,
    content: &str,
    hash: &str,
    config: &Config,
) -> Result<u64, ImportError> {
    info!("Starting full import");

    let records = parse_csv_parallel(content)?;
    let count = do_full_import(db, &records, hash)?;

    save_csv(&config.csv_path(), content).await?;
    save_hash(&config.csv_hash_path(), hash).await?;

    info!("Full import complete: {} records", count);
    Ok(count)
}

pub async fn incremental_import(
    db: &Arc<Database>,
    content: &str,
    hash: &str,
    config: &Config,
) -> Result<(u64, u64, u64), ImportError> {
    info!("Starting incremental import");

    let new_records = parse_csv_parallel(content)?;
    let (added, updated, deleted) = do_incremental_import(db, &new_records, hash)?;

    save_csv(&config.csv_path(), content).await?;
    save_hash(&config.csv_hash_path(), hash).await?;

    info!(
        "Incremental import complete: {} added, {} updated, {} deleted",
        added, updated, deleted
    );
    Ok((added, updated, deleted))
}

pub async fn rebuild_from_csv(db: &Arc<Database>, config: &Config) -> Result<u64, ImportError> {
    info!("Rebuilding database from local CSV");

    let csv_path = config.csv_path();
    if !csv_path.exists() {
        return Err(ImportError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Local CSV not found",
        )));
    }

    let content = load_csv(&csv_path).await?;
    let hash = load_hash(&config.csv_hash_path())
        .await
        .unwrap_or_else(|| compute_hash(&content));

    let records = parse_csv_parallel(&content)?;
    let count = do_full_import(db, &records, &hash)?;

    info!("Database rebuilt: {} records", count);
    Ok(count)
}
