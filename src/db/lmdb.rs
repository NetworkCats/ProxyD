use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use heed::types::{Bytes, SerdeBincode};
use heed::{Database as HeedDb, Env, EnvOpenOptions, RwTxn};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::ip::{IpTrie, MatchVec, ReputationFlags};

#[derive(Error, Debug)]
pub enum DbError {
    #[error("LMDB error: {0}")]
    Heed(#[from] heed::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub last_sync: Option<i64>,
    pub csv_hash: Option<String>,
    pub record_count: u64,
}

pub struct Database {
    env: Env,
    ip_v4: HeedDb<Bytes, SerdeBincode<ReputationFlags>>,
    ip_v6: HeedDb<Bytes, SerdeBincode<ReputationFlags>>,
    cidr_v4: HeedDb<Bytes, SerdeBincode<ReputationFlags>>,
    cidr_v6: HeedDb<Bytes, SerdeBincode<ReputationFlags>>,
    metadata: HeedDb<Bytes, SerdeBincode<Metadata>>,
    cidr_trie: ArcSwap<IpTrie>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Arc<Self>, DbError> {
        std::fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .max_dbs(5)
                .map_size(1024 * 1024 * 1024)
                .open(path)?
        };

        let mut wtxn = env.write_txn()?;
        let ip_v4 = env.create_database(&mut wtxn, Some("ip_v4"))?;
        let ip_v6 = env.create_database(&mut wtxn, Some("ip_v6"))?;
        let cidr_v4 = env.create_database(&mut wtxn, Some("cidr_v4"))?;
        let cidr_v6 = env.create_database(&mut wtxn, Some("cidr_v6"))?;
        let metadata = env.create_database(&mut wtxn, Some("metadata"))?;
        wtxn.commit()?;

        let db = Arc::new(Self {
            env,
            ip_v4,
            ip_v6,
            cidr_v4,
            cidr_v6,
            metadata,
            cidr_trie: ArcSwap::from_pointee(IpTrie::new()),
        });

        db.rebuild_trie()?;

        Ok(db)
    }

    pub fn rebuild_trie(&self) -> Result<(), DbError> {
        let rtxn = self.env.read_txn()?;
        let mut trie = IpTrie::new();

        for result in self.cidr_v4.iter(&rtxn)? {
            let (key, flags) = result?;
            if let Some(network) = key_to_cidr(key) {
                trie.insert(network, flags);
            }
        }

        for result in self.cidr_v6.iter(&rtxn)? {
            let (key, flags) = result?;
            if let Some(network) = key_to_cidr(key) {
                trie.insert(network, flags);
            }
        }

        self.cidr_trie.store(Arc::new(trie));
        Ok(())
    }

    pub fn swap_trie(&self, new_trie: IpTrie) {
        self.cidr_trie.store(Arc::new(new_trie));
    }

    pub fn find_matching_cidrs_fast(&self, ip: IpAddr) -> MatchVec {
        self.cidr_trie.load().find_all_matches(ip)
    }

    pub fn begin_write(&self) -> Result<RwTxn<'_>, DbError> {
        Ok(self.env.write_txn()?)
    }

    pub fn insert_record(
        &self,
        txn: &mut RwTxn,
        entry: &str,
        flags: &ReputationFlags,
    ) -> Result<(), DbError> {
        if let Ok(network) = entry.parse::<IpNetwork>() {
            if network.prefix() == network.ip().max_prefix_len() {
                self.insert_ip(txn, network.ip(), flags)
            } else {
                self.insert_cidr(txn, network, flags)
            }
        } else if let Ok(ip) = entry.parse::<IpAddr>() {
            self.insert_ip(txn, ip, flags)
        } else {
            warn!("Failed to parse entry as IP or CIDR: {}", entry);
            Ok(())
        }
    }

    fn insert_ip(
        &self,
        txn: &mut RwTxn,
        ip: IpAddr,
        flags: &ReputationFlags,
    ) -> Result<(), DbError> {
        match ip {
            IpAddr::V4(v4) => {
                self.ip_v4.put(txn, &v4.octets(), flags)?;
            }
            IpAddr::V6(v6) => {
                self.ip_v6.put(txn, &v6.octets(), flags)?;
            }
        }
        Ok(())
    }

    fn insert_cidr(
        &self,
        txn: &mut RwTxn,
        network: IpNetwork,
        flags: &ReputationFlags,
    ) -> Result<(), DbError> {
        let key = cidr_to_key(network);
        match network {
            IpNetwork::V4(_) => {
                self.cidr_v4.put(txn, key.as_ref(), flags)?;
            }
            IpNetwork::V6(_) => {
                self.cidr_v6.put(txn, key.as_ref(), flags)?;
            }
        }
        Ok(())
    }

    pub fn delete_record(&self, txn: &mut RwTxn, entry: &str) -> Result<bool, DbError> {
        if let Ok(network) = entry.parse::<IpNetwork>() {
            if network.prefix() == network.ip().max_prefix_len() {
                self.delete_ip(txn, network.ip())
            } else {
                self.delete_cidr(txn, network)
            }
        } else if let Ok(ip) = entry.parse::<IpAddr>() {
            self.delete_ip(txn, ip)
        } else {
            Ok(false)
        }
    }

    fn delete_ip(&self, txn: &mut RwTxn, ip: IpAddr) -> Result<bool, DbError> {
        let deleted = match ip {
            IpAddr::V4(v4) => self.ip_v4.delete(txn, &v4.octets())?,
            IpAddr::V6(v6) => self.ip_v6.delete(txn, &v6.octets())?,
        };
        Ok(deleted)
    }

    fn delete_cidr(&self, txn: &mut RwTxn, network: IpNetwork) -> Result<bool, DbError> {
        let key = cidr_to_key(network);
        let deleted = match network {
            IpNetwork::V4(_) => self.cidr_v4.delete(txn, key.as_ref())?,
            IpNetwork::V6(_) => self.cidr_v6.delete(txn, key.as_ref())?,
        };
        Ok(deleted)
    }

    pub fn clear_all(&self, txn: &mut RwTxn) -> Result<(), DbError> {
        self.ip_v4.clear(txn)?;
        self.ip_v6.clear(txn)?;
        self.cidr_v4.clear(txn)?;
        self.cidr_v6.clear(txn)?;
        Ok(())
    }

    pub fn lookup_ip(&self, ip: IpAddr) -> Result<Option<ReputationFlags>, DbError> {
        let rtxn = self.env.read_txn()?;
        match ip {
            IpAddr::V4(v4) => Ok(self.ip_v4.get(&rtxn, &v4.octets())?),
            IpAddr::V6(v6) => Ok(self.ip_v6.get(&rtxn, &v6.octets())?),
        }
    }

    pub fn lookup_ips_batch(
        &self,
        ips: &[IpAddr],
    ) -> Result<Vec<Option<ReputationFlags>>, DbError> {
        let rtxn = self.env.read_txn()?;
        let mut results = Vec::with_capacity(ips.len());

        for ip in ips {
            let flags = match ip {
                IpAddr::V4(v4) => self.ip_v4.get(&rtxn, &v4.octets())?,
                IpAddr::V6(v6) => self.ip_v6.get(&rtxn, &v6.octets())?,
            };
            results.push(flags);
        }

        Ok(results)
    }

    pub fn lookup_cidr(&self, network: IpNetwork) -> Result<Option<ReputationFlags>, DbError> {
        let rtxn = self.env.read_txn()?;
        let key = cidr_to_key(network);
        match network {
            IpNetwork::V4(_) => Ok(self.cidr_v4.get(&rtxn, key.as_ref())?),
            IpNetwork::V6(_) => Ok(self.cidr_v6.get(&rtxn, key.as_ref())?),
        }
    }

    pub fn lookup_cidrs_batch(
        &self,
        networks: &[IpNetwork],
    ) -> Result<Vec<Option<ReputationFlags>>, DbError> {
        let rtxn = self.env.read_txn()?;
        let mut results = Vec::with_capacity(networks.len());

        for network in networks {
            let key = cidr_to_key(*network);
            let flags = match network {
                IpNetwork::V4(_) => self.cidr_v4.get(&rtxn, key.as_ref())?,
                IpNetwork::V6(_) => self.cidr_v6.get(&rtxn, key.as_ref())?,
            };
            results.push(flags);
        }

        Ok(results)
    }

    pub fn get_metadata(&self) -> Result<Metadata, DbError> {
        let rtxn = self.env.read_txn()?;
        Ok(self.metadata.get(&rtxn, b"meta")?.unwrap_or_default())
    }

    pub fn set_metadata(&self, txn: &mut RwTxn, meta: &Metadata) -> Result<(), DbError> {
        self.metadata.put(txn, b"meta", meta)?;
        Ok(())
    }

    pub fn get_all_entries(&self) -> Result<Vec<(String, ReputationFlags)>, DbError> {
        let rtxn = self.env.read_txn()?;
        let mut entries = Vec::new();

        for result in self.ip_v4.iter(&rtxn)? {
            let (key, flags) = result?;
            if key.len() == 4 {
                let octets: [u8; 4] = key.try_into().unwrap();
                let ip = std::net::Ipv4Addr::from(octets);
                entries.push((ip.to_string(), flags));
            }
        }

        for result in self.ip_v6.iter(&rtxn)? {
            let (key, flags) = result?;
            if key.len() == 16 {
                let octets: [u8; 16] = key.try_into().unwrap();
                let ip = std::net::Ipv6Addr::from(octets);
                entries.push((ip.to_string(), flags));
            }
        }

        for result in self.cidr_v4.iter(&rtxn)? {
            let (key, flags) = result?;
            if let Some(network) = key_to_cidr(key) {
                entries.push((network.to_string(), flags));
            }
        }

        for result in self.cidr_v6.iter(&rtxn)? {
            let (key, flags) = result?;
            if let Some(network) = key_to_cidr(key) {
                entries.push((network.to_string(), flags));
            }
        }

        Ok(entries)
    }

    pub fn is_empty(&self) -> Result<bool, DbError> {
        let rtxn = self.env.read_txn()?;
        Ok(self.ip_v4.is_empty(&rtxn)?
            && self.ip_v6.is_empty(&rtxn)?
            && self.cidr_v4.is_empty(&rtxn)?
            && self.cidr_v6.is_empty(&rtxn)?)
    }

    pub fn is_healthy(&self) -> bool {
        self.env.read_txn().is_ok()
    }
}

enum CidrKey {
    V4([u8; 5]),
    V6([u8; 17]),
}

impl AsRef<[u8]> for CidrKey {
    fn as_ref(&self) -> &[u8] {
        match self {
            CidrKey::V4(arr) => arr,
            CidrKey::V6(arr) => arr,
        }
    }
}

fn cidr_to_key(network: IpNetwork) -> CidrKey {
    match network {
        IpNetwork::V4(n) => {
            let octets = n.network().octets();
            CidrKey::V4([octets[0], octets[1], octets[2], octets[3], network.prefix()])
        }
        IpNetwork::V6(n) => {
            let octets = n.network().octets();
            let mut key = [0u8; 17];
            key[..16].copy_from_slice(&octets);
            key[16] = network.prefix();
            CidrKey::V6(key)
        }
    }
}

fn key_to_cidr(key: &[u8]) -> Option<IpNetwork> {
    if key.len() == 5 {
        let octets: [u8; 4] = key[..4].try_into().ok()?;
        let prefix = key[4];
        let addr = std::net::Ipv4Addr::from(octets);
        IpNetwork::new(IpAddr::V4(addr), prefix).ok()
    } else if key.len() == 17 {
        let octets: [u8; 16] = key[..16].try_into().ok()?;
        let prefix = key[16];
        let addr = std::net::Ipv6Addr::from(octets);
        IpNetwork::new(IpAddr::V6(addr), prefix).ok()
    } else {
        None
    }
}

trait IpAddrExt {
    fn max_prefix_len(&self) -> u8;
}

impl IpAddrExt for IpAddr {
    fn max_prefix_len(&self) -> u8 {
        match self {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (TempDir, Arc<Database>) {
        let dir = TempDir::new().unwrap();
        let db = Database::open(dir.path()).unwrap();
        (dir, db)
    }

    #[test]
    fn test_insert_and_lookup_ipv4() {
        let (_dir, db) = create_test_db();
        let flags = ReputationFlags {
            proxy: true,
            ..Default::default()
        };

        let mut txn = db.begin_write().unwrap();
        db.insert_record(&mut txn, "192.168.1.1", &flags).unwrap();
        txn.commit().unwrap();

        let result = db.lookup_ip("192.168.1.1".parse().unwrap()).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().proxy);
    }

    #[test]
    fn test_insert_and_lookup_cidr() {
        let (_dir, db) = create_test_db();
        let flags = ReputationFlags {
            cdn: true,
            ..Default::default()
        };

        let mut txn = db.begin_write().unwrap();
        db.insert_record(&mut txn, "10.0.0.0/8", &flags).unwrap();
        txn.commit().unwrap();

        db.rebuild_trie().unwrap();
        let matches = db.find_matching_cidrs_fast("10.1.2.3".parse().unwrap());
        assert_eq!(matches.len(), 1);
        assert!(matches[0].1.cdn);
    }

    #[test]
    fn test_ipv6_support() {
        let (_dir, db) = create_test_db();
        let flags = ReputationFlags {
            tor: true,
            ..Default::default()
        };

        let mut txn = db.begin_write().unwrap();
        db.insert_record(&mut txn, "2001:db8::1", &flags).unwrap();
        db.insert_record(&mut txn, "2001:db8::/32", &flags).unwrap();
        txn.commit().unwrap();

        let result = db.lookup_ip("2001:db8::1".parse().unwrap()).unwrap();
        assert!(result.is_some());

        db.rebuild_trie().unwrap();
        let matches = db.find_matching_cidrs_fast("2001:db8::2".parse().unwrap());
        assert_eq!(matches.len(), 1);
    }
}
