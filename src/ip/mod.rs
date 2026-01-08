mod matcher;
mod trie;

pub use matcher::{
    lookup_ip, lookup_ips_batch, lookup_range, lookup_ranges_batch, LookupError, LookupResult,
    MatchedEntry, ReputationFlags,
};
pub use trie::IpTrie;
