use std::net::IpAddr;
use std::sync::Arc;

use tempfile::TempDir;

mod common {
    use super::*;

    pub struct TestContext {
        pub db: Arc<proxyd::db::Database>,
        #[allow(dead_code)]
        dir: TempDir,
    }

    impl TestContext {
        pub fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp directory");
            let db = proxyd::db::Database::open(dir.path()).expect("failed to open database");
            Self { db, dir }
        }

        pub fn insert_ip(&self, ip: &str, flags: proxyd::ip::ReputationFlags) {
            let mut txn = self.db.begin_write().unwrap();
            self.db.insert_record(&mut txn, ip, &flags).unwrap();
            txn.commit().unwrap();
        }

        pub fn insert_cidr(&self, cidr: &str, flags: proxyd::ip::ReputationFlags) {
            let mut txn = self.db.begin_write().unwrap();
            self.db.insert_record(&mut txn, cidr, &flags).unwrap();
            txn.commit().unwrap();
            self.db.rebuild_trie().unwrap();
        }

        pub fn insert_records(&self, records: &[(&str, proxyd::ip::ReputationFlags)]) {
            let mut txn = self.db.begin_write().unwrap();
            for (entry, flags) in records {
                self.db.insert_record(&mut txn, entry, flags).unwrap();
            }
            txn.commit().unwrap();
            self.db.rebuild_trie().unwrap();
        }
    }
}

use common::TestContext;

mod database_tests {
    use super::*;

    #[test]
    fn single_ip_lookup() {
        let ctx = TestContext::new();

        let flags = proxyd::ip::ReputationFlags {
            proxy: true,
            vpn: true,
            ..Default::default()
        };
        ctx.insert_ip("192.168.1.100", flags);

        let result = proxyd::ip::lookup_ip(&ctx.db, "192.168.1.100").unwrap();

        assert!(result.found, "expected IP to be found");
        assert_eq!(result.query, "192.168.1.100");
        assert!(result.flags.proxy, "expected proxy flag to be set");
        assert!(result.flags.vpn, "expected vpn flag to be set");
        assert!(!result.flags.tor, "expected tor flag to be unset");
        assert_eq!(result.matched_entries.len(), 1);
    }

    #[test]
    fn cidr_trie_lookup() {
        let ctx = TestContext::new();

        let flags = proxyd::ip::ReputationFlags {
            cdn: true,
            webhost: true,
            ..Default::default()
        };
        ctx.insert_cidr("10.0.0.0/8", flags);

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.50.100.200").unwrap();

        assert!(result.found, "expected IP within CIDR to be found");
        assert!(result.flags.cdn, "expected cdn flag");
        assert!(result.flags.webhost, "expected webhost flag");
        assert_eq!(result.matched_entries.len(), 1);
        assert_eq!(result.matched_entries[0].entry, "10.0.0.0/8");
    }

    #[test]
    fn combined_ip_and_cidr_lookup() {
        let ctx = TestContext::new();

        let cidr_flags = proxyd::ip::ReputationFlags {
            cdn: true,
            ..Default::default()
        };
        ctx.insert_cidr("172.16.0.0/12", cidr_flags);

        let ip_flags = proxyd::ip::ReputationFlags {
            proxy: true,
            ..Default::default()
        };
        ctx.insert_ip("172.16.1.1", ip_flags);

        let result = proxyd::ip::lookup_ip(&ctx.db, "172.16.1.1").unwrap();

        assert!(result.found);
        assert!(result.flags.cdn, "expected cdn flag from CIDR");
        assert!(result.flags.proxy, "expected proxy flag from exact IP");
        assert_eq!(result.matched_entries.len(), 2, "expected both matches");
    }

    #[test]
    fn nested_cidr_matching() {
        let ctx = TestContext::new();

        let broad_flags = proxyd::ip::ReputationFlags {
            rangeblock: true,
            ..Default::default()
        };

        let narrow_flags = proxyd::ip::ReputationFlags {
            tor: true,
            ..Default::default()
        };

        ctx.insert_records(&[
            ("192.0.0.0/8", broad_flags),
            ("192.168.0.0/16", narrow_flags),
        ]);

        let result = proxyd::ip::lookup_ip(&ctx.db, "192.168.100.50").unwrap();

        assert!(result.found);
        assert!(result.flags.rangeblock, "expected broad CIDR flag");
        assert!(result.flags.tor, "expected narrow CIDR flag");
        assert_eq!(result.matched_entries.len(), 2);
    }

    #[test]
    fn clear_and_reimport() {
        let ctx = TestContext::new();

        ctx.insert_ip(
            "192.168.1.1",
            proxyd::ip::ReputationFlags {
                proxy: true,
                ..Default::default()
            },
        );

        assert!(proxyd::ip::lookup_ip(&ctx.db, "192.168.1.1").unwrap().found);

        {
            let mut txn = ctx.db.begin_write().unwrap();
            ctx.db.clear_all(&mut txn).unwrap();
            txn.commit().unwrap();
            ctx.db.rebuild_trie().unwrap();
        }

        assert!(!proxyd::ip::lookup_ip(&ctx.db, "192.168.1.1").unwrap().found);

        ctx.insert_ip(
            "10.0.0.1",
            proxyd::ip::ReputationFlags {
                cdn: true,
                ..Default::default()
            },
        );

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.0.0.1").unwrap();
        assert!(result.found);
        assert!(result.flags.cdn);
    }

    #[test]
    fn delete_record() {
        let ctx = TestContext::new();

        ctx.insert_ip(
            "192.168.1.1",
            proxyd::ip::ReputationFlags {
                proxy: true,
                ..Default::default()
            },
        );

        assert!(proxyd::ip::lookup_ip(&ctx.db, "192.168.1.1").unwrap().found);

        {
            let mut txn = ctx.db.begin_write().unwrap();
            let deleted = ctx.db.delete_record(&mut txn, "192.168.1.1").unwrap();
            assert!(deleted, "expected record to be deleted");
            txn.commit().unwrap();
        }

        assert!(!proxyd::ip::lookup_ip(&ctx.db, "192.168.1.1").unwrap().found);
    }

    #[test]
    fn metadata_persistence() {
        let ctx = TestContext::new();

        let meta = proxyd::db::Metadata {
            last_sync: Some(1700000000),
            csv_hash: Some("abc123".to_owned()),
            record_count: 1000,
        };

        {
            let mut txn = ctx.db.begin_write().unwrap();
            ctx.db.set_metadata(&mut txn, &meta).unwrap();
            txn.commit().unwrap();
        }

        let retrieved = ctx.db.get_metadata().unwrap();
        assert_eq!(retrieved.last_sync, Some(1700000000));
        assert_eq!(retrieved.csv_hash, Some("abc123".to_owned()));
        assert_eq!(retrieved.record_count, 1000);
    }

    #[test]
    fn get_all_entries() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "192.168.1.1",
                proxyd::ip::ReputationFlags {
                    proxy: true,
                    ..Default::default()
                },
            ),
            (
                "10.0.0.0/8",
                proxyd::ip::ReputationFlags {
                    cdn: true,
                    ..Default::default()
                },
            ),
            (
                "2001:db8::1",
                proxyd::ip::ReputationFlags {
                    tor: true,
                    ..Default::default()
                },
            ),
        ]);

        let entries = ctx.db.get_all_entries().unwrap();
        assert_eq!(entries.len(), 3);

        let entry_strs: Vec<&str> = entries.iter().map(|(s, _)| s.as_str()).collect();
        assert!(entry_strs.contains(&"192.168.1.1"));
        assert!(entry_strs.contains(&"10.0.0.0/8"));
        assert!(entry_strs.contains(&"2001:db8::1"));
    }

    #[test]
    fn health_check() {
        let ctx = TestContext::new();
        assert!(ctx.db.is_healthy());
    }
}

mod batch_tests {
    use super::*;

    #[test]
    fn batch_ip_lookup() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "8.8.8.8",
                proxyd::ip::ReputationFlags {
                    proxy: true,
                    ..Default::default()
                },
            ),
            (
                "1.1.1.1",
                proxyd::ip::ReputationFlags {
                    cdn: true,
                    ..Default::default()
                },
            ),
            (
                "9.9.9.9",
                proxyd::ip::ReputationFlags {
                    vpn: true,
                    ..Default::default()
                },
            ),
        ]);

        let ips = vec!["8.8.8.8", "1.1.1.1", "9.9.9.9", "203.0.113.1"];
        let results = proxyd::ip::lookup_ips_batch(&ctx.db, &ips).unwrap();

        assert_eq!(results.len(), 4);
        assert!(results[0].found && results[0].flags.proxy);
        assert!(results[1].found && results[1].flags.cdn);
        assert!(results[2].found && results[2].flags.vpn);
        assert!(!results[3].found);
    }

    #[test]
    fn batch_range_lookup() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "10.0.0.0/8",
                proxyd::ip::ReputationFlags {
                    anonblock: true,
                    ..Default::default()
                },
            ),
            (
                "172.16.0.0/12",
                proxyd::ip::ReputationFlags {
                    school_block: true,
                    ..Default::default()
                },
            ),
        ]);

        let cidrs = vec!["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"];
        let results = proxyd::ip::lookup_ranges_batch(&ctx.db, &cidrs).unwrap();

        assert_eq!(results.len(), 3);
        assert!(results[0].found && results[0].flags.anonblock);
        assert!(results[1].found && results[1].flags.school_block);
        assert!(!results[2].found);
    }

    #[test]
    fn empty_batch() {
        let ctx = TestContext::new();

        let empty_ips: Vec<&str> = vec![];
        let results = proxyd::ip::lookup_ips_batch(&ctx.db, &empty_ips).unwrap();
        assert!(results.is_empty());

        let empty_cidrs: Vec<&str> = vec![];
        let results = proxyd::ip::lookup_ranges_batch(&ctx.db, &empty_cidrs).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn single_item_batch() {
        let ctx = TestContext::new();

        ctx.insert_ip(
            "1.2.3.4",
            proxyd::ip::ReputationFlags {
                tor: true,
                ..Default::default()
            },
        );

        let ips = vec!["1.2.3.4"];
        let results = proxyd::ip::lookup_ips_batch(&ctx.db, &ips).unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].found);
        assert!(results[0].flags.tor);
    }

    #[test]
    fn batch_all_not_found() {
        let ctx = TestContext::new();

        let ips = vec!["1.1.1.1", "2.2.2.2", "3.3.3.3"];
        let results = proxyd::ip::lookup_ips_batch(&ctx.db, &ips).unwrap();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| !r.found));
    }
}

mod ipv6_tests {
    use super::*;

    #[test]
    fn ipv6_exact_and_cidr() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "2001:db8::1",
                proxyd::ip::ReputationFlags {
                    tor: true,
                    ..Default::default()
                },
            ),
            (
                "2001:db8::/32",
                proxyd::ip::ReputationFlags {
                    vpn: true,
                    ..Default::default()
                },
            ),
        ]);

        // Exact match + CIDR match
        let result = proxyd::ip::lookup_ip(&ctx.db, "2001:db8::1").unwrap();
        assert!(result.found);
        assert!(result.flags.tor);
        assert!(result.flags.vpn);

        // CIDR match only
        let result = proxyd::ip::lookup_ip(&ctx.db, "2001:db8::2").unwrap();
        assert!(result.found);
        assert!(!result.flags.tor);
        assert!(result.flags.vpn);

        // No match
        let result = proxyd::ip::lookup_ip(&ctx.db, "2001:db9::1").unwrap();
        assert!(!result.found);
    }

    #[test]
    fn ipv6_full_address() {
        let ctx = TestContext::new();

        ctx.insert_ip(
            "2001:0db8:0000:0000:0000:0000:0000:0001",
            proxyd::ip::ReputationFlags {
                proxy: true,
                ..Default::default()
            },
        );

        // Should match compressed form
        let result = proxyd::ip::lookup_ip(&ctx.db, "2001:db8::1").unwrap();
        assert!(result.found);
        assert!(result.flags.proxy);
    }

    #[test]
    fn ipv6_batch() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "2001:db8::1",
                proxyd::ip::ReputationFlags {
                    cdn: true,
                    ..Default::default()
                },
            ),
            (
                "fe80::/10",
                proxyd::ip::ReputationFlags {
                    rangeblock: true,
                    ..Default::default()
                },
            ),
        ]);

        let ips = vec!["2001:db8::1", "fe80::1", "::1"];
        let results = proxyd::ip::lookup_ips_batch(&ctx.db, &ips).unwrap();

        assert_eq!(results.len(), 3);
        assert!(results[0].found && results[0].flags.cdn);
        assert!(results[1].found && results[1].flags.rangeblock);
        assert!(!results[2].found);
    }
}

mod trie_tests {
    use super::*;

    #[test]
    fn trie_fast_lookup_with_many_entries() {
        let ctx = TestContext::new();

        {
            let mut txn = ctx.db.begin_write().unwrap();
            for i in 0..100u8 {
                let cidr = format!("{}.0.0.0/8", i);
                ctx.db
                    .insert_record(
                        &mut txn,
                        &cidr,
                        &proxyd::ip::ReputationFlags {
                            proxy: true,
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
            txn.commit().unwrap();
            ctx.db.rebuild_trie().unwrap();
        }

        for i in 0..100u8 {
            let ip_str = format!("{}.1.2.3", i);
            let ip: IpAddr = ip_str.parse().unwrap();
            let matches = ctx.db.find_matching_cidrs_fast(ip);
            assert_eq!(matches.len(), 1, "expected single match for {}", ip_str);
        }

        let no_match: IpAddr = "200.1.2.3".parse().unwrap();
        let matches = ctx.db.find_matching_cidrs_fast(no_match);
        assert!(matches.is_empty());
    }

    #[test]
    fn trie_overlapping_cidrs() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "10.0.0.0/8",
                proxyd::ip::ReputationFlags {
                    anonblock: true,
                    ..Default::default()
                },
            ),
            (
                "10.10.0.0/16",
                proxyd::ip::ReputationFlags {
                    proxy: true,
                    ..Default::default()
                },
            ),
            (
                "10.10.10.0/24",
                proxyd::ip::ReputationFlags {
                    vpn: true,
                    ..Default::default()
                },
            ),
        ]);

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.10.10.5").unwrap();
        assert!(result.found);
        assert_eq!(result.matched_entries.len(), 3, "expected all 3 CIDRs to match");
        assert!(result.flags.anonblock);
        assert!(result.flags.proxy);
        assert!(result.flags.vpn);

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.10.20.1").unwrap();
        assert!(result.found);
        assert_eq!(result.matched_entries.len(), 2, "expected 2 CIDRs to match");
        assert!(result.flags.anonblock);
        assert!(result.flags.proxy);
        assert!(!result.flags.vpn);

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.20.1.1").unwrap();
        assert!(result.found);
        assert_eq!(result.matched_entries.len(), 1);
        assert!(result.flags.anonblock);
    }
}

mod error_tests {
    use super::*;

    #[test]
    fn invalid_ip_address() {
        let ctx = TestContext::new();

        assert!(proxyd::ip::lookup_ip(&ctx.db, "not-an-ip").is_err());
        assert!(proxyd::ip::lookup_ip(&ctx.db, "256.256.256.256").is_err());
        assert!(proxyd::ip::lookup_ip(&ctx.db, "999.999.999.999").is_err());
        assert!(proxyd::ip::lookup_ip(&ctx.db, "").is_err());
        assert!(proxyd::ip::lookup_ip(&ctx.db, "192.168.1").is_err());
        assert!(proxyd::ip::lookup_ip(&ctx.db, "192.168.1.1.1").is_err());
    }

    #[test]
    fn invalid_cidr() {
        let ctx = TestContext::new();

        assert!(proxyd::ip::lookup_range(&ctx.db, "not-a-cidr").is_err());
        assert!(proxyd::ip::lookup_range(&ctx.db, "192.168.1.0/33").is_err());
        assert!(proxyd::ip::lookup_range(&ctx.db, "192.168.1.0/-1").is_err());
        assert!(proxyd::ip::lookup_range(&ctx.db, "").is_err());
    }

    #[test]
    fn batch_with_invalid_entry() {
        let ctx = TestContext::new();

        let ips = vec!["1.1.1.1", "invalid", "2.2.2.2"];
        let result = proxyd::ip::lookup_ips_batch(&ctx.db, &ips);
        assert!(result.is_err(), "batch should fail on invalid IP");

        let cidrs = vec!["10.0.0.0/8", "invalid/cidr"];
        let result = proxyd::ip::lookup_ranges_batch(&ctx.db, &cidrs);
        assert!(result.is_err(), "batch should fail on invalid CIDR");
    }

    #[test]
    fn delete_nonexistent_record() {
        let ctx = TestContext::new();

        let mut txn = ctx.db.begin_write().unwrap();
        let deleted = ctx.db.delete_record(&mut txn, "192.168.1.1").unwrap();
        assert!(!deleted, "expected delete to return false for nonexistent record");
        txn.commit().unwrap();
    }
}

mod concurrency_tests {
    use super::*;
    use std::thread;

    #[test]
    fn concurrent_reads() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "1.1.1.1",
                proxyd::ip::ReputationFlags {
                    proxy: true,
                    ..Default::default()
                },
            ),
            (
                "10.0.0.0/8",
                proxyd::ip::ReputationFlags {
                    vpn: true,
                    ..Default::default()
                },
            ),
        ]);

        let db = ctx.db.clone();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let db = db.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        let result = proxyd::ip::lookup_ip(&db, "1.1.1.1").unwrap();
                        assert!(result.found);

                        let result = proxyd::ip::lookup_ip(&db, "10.1.2.3").unwrap();
                        assert!(result.found);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }

    #[test]
    fn concurrent_batch_reads() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "8.8.8.8",
                proxyd::ip::ReputationFlags {
                    cdn: true,
                    ..Default::default()
                },
            ),
            (
                "1.1.1.1",
                proxyd::ip::ReputationFlags {
                    proxy: true,
                    ..Default::default()
                },
            ),
        ]);

        let db = ctx.db.clone();
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let db = db.clone();
                thread::spawn(move || {
                    for _ in 0..50 {
                        let ips = vec!["8.8.8.8", "1.1.1.1", "2.2.2.2"];
                        let results = proxyd::ip::lookup_ips_batch(&db, &ips).unwrap();
                        assert_eq!(results.len(), 3);
                        assert!(results[0].found);
                        assert!(results[1].found);
                        assert!(!results[2].found);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread panicked");
        }
    }
}

mod flags_tests {
    use super::*;

    #[test]
    fn all_flags_set() {
        let ctx = TestContext::new();

        let all_flags = proxyd::ip::ReputationFlags {
            anonblock: true,
            proxy: true,
            vpn: true,
            cdn: true,
            public_wifi: true,
            rangeblock: true,
            school_block: true,
            tor: true,
            webhost: true,
        };
        ctx.insert_ip("1.2.3.4", all_flags);

        let result = proxyd::ip::lookup_ip(&ctx.db, "1.2.3.4").unwrap();
        assert!(result.found);
        assert!(result.flags.anonblock);
        assert!(result.flags.proxy);
        assert!(result.flags.vpn);
        assert!(result.flags.cdn);
        assert!(result.flags.public_wifi);
        assert!(result.flags.rangeblock);
        assert!(result.flags.school_block);
        assert!(result.flags.tor);
        assert!(result.flags.webhost);
    }

    #[test]
    fn flags_merge_from_multiple_sources() {
        let ctx = TestContext::new();

        ctx.insert_records(&[
            (
                "10.0.0.0/8",
                proxyd::ip::ReputationFlags {
                    anonblock: true,
                    proxy: true,
                    ..Default::default()
                },
            ),
            (
                "10.10.0.0/16",
                proxyd::ip::ReputationFlags {
                    vpn: true,
                    cdn: true,
                    ..Default::default()
                },
            ),
            (
                "10.10.10.10",
                proxyd::ip::ReputationFlags {
                    tor: true,
                    webhost: true,
                    ..Default::default()
                },
            ),
        ]);

        let result = proxyd::ip::lookup_ip(&ctx.db, "10.10.10.10").unwrap();
        assert!(result.found);
        assert_eq!(result.matched_entries.len(), 3);

        // Merged flags from all sources
        assert!(result.flags.anonblock);
        assert!(result.flags.proxy);
        assert!(result.flags.vpn);
        assert!(result.flags.cdn);
        assert!(result.flags.tor);
        assert!(result.flags.webhost);

        // Not set by any source
        assert!(!result.flags.public_wifi);
        assert!(!result.flags.rangeblock);
        assert!(!result.flags.school_block);
    }
}
