use std::net::IpAddr;
use std::sync::Arc;

use ipnetwork::IpNetwork;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::db::{Database, DbError};

#[derive(Error, Debug)]
pub enum LookupError {
    #[error("Invalid IP address: {0}")]
    InvalidIp(String),
    #[error("Invalid CIDR notation: {0}")]
    InvalidCidr(String),
    #[error("Database error: {0}")]
    Database(#[from] DbError),
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct ReputationFlags {
    pub anonblock: bool,
    pub proxy: bool,
    pub vpn: bool,
    pub cdn: bool,
    pub public_wifi: bool,
    pub rangeblock: bool,
    pub school_block: bool,
    pub tor: bool,
    pub webhost: bool,
}

impl ReputationFlags {
    pub fn merge(&self, other: &ReputationFlags) -> ReputationFlags {
        ReputationFlags {
            anonblock: self.anonblock || other.anonblock,
            proxy: self.proxy || other.proxy,
            vpn: self.vpn || other.vpn,
            cdn: self.cdn || other.cdn,
            public_wifi: self.public_wifi || other.public_wifi,
            rangeblock: self.rangeblock || other.rangeblock,
            school_block: self.school_block || other.school_block,
            tor: self.tor || other.tor,
            webhost: self.webhost || other.webhost,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedEntry {
    pub entry: String,
    pub flags: ReputationFlags,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResult {
    pub found: bool,
    pub query: String,
    pub flags: ReputationFlags,
    pub matched_entries: Vec<MatchedEntry>,
}

pub fn lookup_ip(db: &Arc<Database>, ip_str: &str) -> Result<LookupResult, LookupError> {
    let ip: IpAddr = ip_str
        .parse()
        .map_err(|_| LookupError::InvalidIp(ip_str.to_owned()))?;

    let mut matched_entries = Vec::new();
    let mut merged_flags = ReputationFlags::default();

    if let Some(flags) = db.lookup_ip(ip)? {
        matched_entries.push(MatchedEntry {
            entry: ip.to_string(),
            flags,
        });
        merged_flags = merged_flags.merge(&flags);
    }

    for (network, flags) in db.find_matching_cidrs_fast(ip) {
        matched_entries.push(MatchedEntry {
            entry: network.to_string(),
            flags,
        });
        merged_flags = merged_flags.merge(&flags);
    }

    Ok(LookupResult {
        found: !matched_entries.is_empty(),
        query: ip_str.to_owned(),
        flags: merged_flags,
        matched_entries,
    })
}

pub fn lookup_range(db: &Arc<Database>, cidr_str: &str) -> Result<LookupResult, LookupError> {
    let network: IpNetwork = cidr_str
        .parse()
        .map_err(|_| LookupError::InvalidCidr(cidr_str.to_owned()))?;

    let mut matched_entries = Vec::new();

    if let Some(flags) = db.lookup_cidr(network)? {
        matched_entries.push(MatchedEntry {
            entry: network.to_string(),
            flags,
        });
    }

    let merged_flags = matched_entries
        .iter()
        .fold(ReputationFlags::default(), |acc, e| acc.merge(&e.flags));

    Ok(LookupResult {
        found: !matched_entries.is_empty(),
        query: cidr_str.to_owned(),
        flags: merged_flags,
        matched_entries,
    })
}

pub fn lookup_ips_batch(
    db: &Arc<Database>,
    ip_strs: &[&str],
) -> Result<Vec<LookupResult>, LookupError> {
    let ips: Vec<IpAddr> = ip_strs
        .iter()
        .map(|s| {
            s.parse()
                .map_err(|_| LookupError::InvalidIp((*s).to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let db_results = db.lookup_ips_batch(&ips)?;

    let results: Vec<LookupResult> = ips
        .par_iter()
        .zip(db_results.par_iter())
        .zip(ip_strs.par_iter())
        .map(|((ip, db_result), query)| {
            let mut matched_entries = Vec::new();
            let mut merged_flags = ReputationFlags::default();

            if let Some(flags) = db_result {
                matched_entries.push(MatchedEntry {
                    entry: ip.to_string(),
                    flags: *flags,
                });
                merged_flags = merged_flags.merge(flags);
            }

            for (network, flags) in db.find_matching_cidrs_fast(*ip) {
                matched_entries.push(MatchedEntry {
                    entry: network.to_string(),
                    flags,
                });
                merged_flags = merged_flags.merge(&flags);
            }

            LookupResult {
                found: !matched_entries.is_empty(),
                query: (*query).to_owned(),
                flags: merged_flags,
                matched_entries,
            }
        })
        .collect();

    Ok(results)
}

pub fn lookup_ranges_batch(
    db: &Arc<Database>,
    cidr_strs: &[&str],
) -> Result<Vec<LookupResult>, LookupError> {
    let networks: Vec<IpNetwork> = cidr_strs
        .iter()
        .map(|s| {
            s.parse()
                .map_err(|_| LookupError::InvalidCidr((*s).to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let db_results = db.lookup_cidrs_batch(&networks)?;

    let results: Vec<LookupResult> = networks
        .par_iter()
        .zip(db_results.par_iter())
        .zip(cidr_strs.par_iter())
        .map(|((network, db_result), query)| {
            let mut matched_entries = Vec::new();

            if let Some(flags) = db_result {
                matched_entries.push(MatchedEntry {
                    entry: network.to_string(),
                    flags: *flags,
                });
            }

            let merged_flags = matched_entries
                .iter()
                .fold(ReputationFlags::default(), |acc, e| acc.merge(&e.flags));

            LookupResult {
                found: !matched_entries.is_empty(),
                query: (*query).to_owned(),
                flags: merged_flags,
                matched_entries,
            }
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reputation_flags_merge() {
        let a = ReputationFlags {
            proxy: true,
            ..Default::default()
        };
        let b = ReputationFlags {
            vpn: true,
            ..Default::default()
        };
        let merged = a.merge(&b);
        assert!(merged.proxy);
        assert!(merged.vpn);
        assert!(!merged.tor);
    }

    #[test]
    fn test_lookup_error_display() {
        let err = LookupError::InvalidIp("not-an-ip".to_owned());
        assert!(err.to_string().contains("Invalid IP"));

        let err = LookupError::InvalidCidr("not-a-cidr".to_owned());
        assert!(err.to_string().contains("Invalid CIDR"));
    }
}
