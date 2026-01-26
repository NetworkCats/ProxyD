use std::net::IpAddr;

use ipnetwork::IpNetwork;
use smallvec::SmallVec;

use super::ReputationFlags;

pub type MatchVec = SmallVec<[(IpNetwork, ReputationFlags); 4]>;

struct PatriciaNode {
    prefix_bits: u128,
    prefix_len: u8,
    data: Option<(IpNetwork, ReputationFlags)>,
    children: [Option<Box<PatriciaNode>>; 2],
}

impl PatriciaNode {
    fn new(prefix_bits: u128, prefix_len: u8) -> Self {
        Self {
            prefix_bits,
            prefix_len,
            data: None,
            children: [None, None],
        }
    }

    fn new_leaf(prefix_bits: u128, prefix_len: u8, network: IpNetwork, flags: ReputationFlags) -> Self {
        Self {
            prefix_bits,
            prefix_len,
            data: Some((network, flags)),
            children: [None, None],
        }
    }
}

pub struct IpTrie {
    v4_root: Option<Box<PatriciaNode>>,
    v6_root: Option<Box<PatriciaNode>>,
}

impl Default for IpTrie {
    fn default() -> Self {
        Self::new()
    }
}

impl IpTrie {
    pub fn new() -> Self {
        Self {
            v4_root: None,
            v6_root: None,
        }
    }

    pub fn insert(&mut self, network: IpNetwork, flags: ReputationFlags) {
        match network {
            IpNetwork::V4(n) => {
                let bits = u128::from(u32::from(n.network()));
                let prefix = n.prefix();
                Self::insert_node(&mut self.v4_root, bits, prefix, 32, network, flags);
            }
            IpNetwork::V6(n) => {
                let bits = u128::from(n.network());
                let prefix = n.prefix();
                Self::insert_node(&mut self.v6_root, bits, prefix, 128, network, flags);
            }
        }
    }

    fn insert_node(
        root: &mut Option<Box<PatriciaNode>>,
        bits: u128,
        prefix_len: u8,
        total_bits: u8,
        network: IpNetwork,
        flags: ReputationFlags,
    ) {
        if root.is_none() {
            *root = Some(Box::new(PatriciaNode::new_leaf(bits, prefix_len, network, flags)));
            return;
        }

        let node = root.as_mut().unwrap();
        let common_len = Self::common_prefix_len(
            node.prefix_bits,
            bits,
            node.prefix_len.min(prefix_len),
            total_bits,
        );

        if common_len == node.prefix_len && common_len == prefix_len {
            node.data = Some((network, flags));
            return;
        }

        if common_len == node.prefix_len {
            let child_bit = Self::get_bit(bits, common_len, total_bits);
            Self::insert_node(
                &mut node.children[child_bit],
                bits,
                prefix_len,
                total_bits,
                network,
                flags,
            );
            return;
        }

        let old_node = root.take().unwrap();
        let common_prefix_bits = Self::mask_prefix(bits, common_len, total_bits);
        let mut new_parent = Box::new(PatriciaNode::new(common_prefix_bits, common_len));

        if common_len == prefix_len {
            new_parent.data = Some((network, flags));
            let old_bit = Self::get_bit(old_node.prefix_bits, common_len, total_bits);
            new_parent.children[old_bit] = Some(old_node);
        } else {
            let new_bit = Self::get_bit(bits, common_len, total_bits);
            let old_bit = 1 - new_bit;

            new_parent.children[new_bit] =
                Some(Box::new(PatriciaNode::new_leaf(bits, prefix_len, network, flags)));
            new_parent.children[old_bit] = Some(old_node);
        }

        *root = Some(new_parent);
    }

    fn common_prefix_len(a: u128, b: u128, max_len: u8, total_bits: u8) -> u8 {
        if max_len == 0 {
            return 0;
        }

        let shift = total_bits.saturating_sub(max_len);
        let diff = (a >> shift) ^ (b >> shift);

        if diff == 0 {
            max_len
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let leading = diff.leading_zeros() as u8;
            leading.saturating_sub(128 - max_len).min(max_len)
        }
    }

    fn get_bit(bits: u128, pos: u8, total_bits: u8) -> usize {
        let shift = total_bits.saturating_sub(pos + 1);
        ((bits >> shift) & 1) as usize
    }

    fn mask_prefix(bits: u128, prefix_len: u8, total_bits: u8) -> u128 {
        if prefix_len == 0 {
            return 0;
        }
        let shift = total_bits.saturating_sub(prefix_len);
        if shift >= 128 {
            0
        } else {
            (bits >> shift) << shift
        }
    }

    pub fn find_all_matches(&self, ip: IpAddr) -> MatchVec {
        match ip {
            IpAddr::V4(v4) => self.find_matches_impl(&self.v4_root, u128::from(u32::from(v4)), 32),
            IpAddr::V6(v6) => self.find_matches_impl(&self.v6_root, u128::from(v6), 128),
        }
    }

    #[allow(clippy::ref_option, clippy::unused_self)]
    fn find_matches_impl(
        &self,
        root: &Option<Box<PatriciaNode>>,
        ip_bits: u128,
        total_bits: u8,
    ) -> MatchVec {
        let mut matches = MatchVec::new();
        let mut current = root;

        while let Some(node) = current {
            let common =
                Self::common_prefix_len(node.prefix_bits, ip_bits, node.prefix_len, total_bits);
            if common < node.prefix_len {
                break;
            }

            if let Some((network, flags)) = &node.data {
                matches.push((*network, *flags));
            }

            if node.prefix_len >= total_bits {
                break;
            }

            let child_bit = Self::get_bit(ip_bits, node.prefix_len, total_bits);
            current = &node.children[child_bit];
        }

        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_find_v4() {
        let mut trie = IpTrie::new();
        let flags = ReputationFlags {
            proxy: true,
            ..Default::default()
        };

        trie.insert("10.0.0.0/8".parse().unwrap(), flags);

        let matches = trie.find_all_matches("10.1.2.3".parse().unwrap());
        assert_eq!(matches.len(), 1);
        assert!(matches[0].1.proxy);

        let no_matches = trie.find_all_matches("192.168.1.1".parse().unwrap());
        assert!(no_matches.is_empty());
    }

    #[test]
    fn test_multiple_matches() {
        let mut trie = IpTrie::new();

        trie.insert(
            "10.0.0.0/8".parse().unwrap(),
            ReputationFlags {
                proxy: true,
                ..Default::default()
            },
        );
        trie.insert(
            "10.0.0.0/16".parse().unwrap(),
            ReputationFlags {
                vpn: true,
                ..Default::default()
            },
        );

        let matches = trie.find_all_matches("10.0.1.1".parse().unwrap());
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_v6() {
        let mut trie = IpTrie::new();
        let flags = ReputationFlags {
            tor: true,
            ..Default::default()
        };

        trie.insert("2001:db8::/32".parse().unwrap(), flags);

        let matches = trie.find_all_matches("2001:db8::1".parse().unwrap());
        assert_eq!(matches.len(), 1);
        assert!(matches[0].1.tor);
    }

    #[test]
    fn test_exact_match() {
        let mut trie = IpTrie::new();
        let flags = ReputationFlags {
            cdn: true,
            ..Default::default()
        };

        trie.insert("192.168.1.0/24".parse().unwrap(), flags);
        trie.insert("192.168.1.100/32".parse().unwrap(), flags);

        let matches = trie.find_all_matches("192.168.1.100".parse().unwrap());
        assert_eq!(matches.len(), 2);
    }
}
