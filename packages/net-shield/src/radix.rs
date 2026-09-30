use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAction {
    Block,
    Allow,
    Proxy,
}

// ---------------------------------------------------------------------------
// DomainFilter — label trie, root-inward (TLD first in storage)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct TrieNode {
    children: HashMap<String, TrieNode>,
    action: Option<FilterAction>,
}

#[derive(Debug, Default)]
pub struct DomainFilter {
    root: TrieNode,
}

impl DomainFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> Self {
        let mut filter = DomainFilter::default();
        for (domain, action) in rules {
            filter.insert(domain, action.clone());
        }
        filter
    }

    /// **Not normalized.** `domain` is stored exactly as given — no case-folding, no trailing-dot
    /// stripping, no IDNA. This was already true before module 6 of the domain-blocklist plan; that
    /// module's precedence resolver now queries `rule_for` with a *normalized* key
    /// (`domain_normalize::normalize`), so a rule inserted with a differently-spelled domain (mixed
    /// case, a trailing dot, a non-ASCII IDN label) becomes silently unreachable at level 2 — it
    /// falls through to the FST/default instead of matching, rather than erroring. No caller
    /// currently builds a `DomainFilter` from untrusted/raw config strings (`cli`, module 7 of the
    /// domain-blocklist plan, is unbuilt), so this hazard is real but dormant; whichever module
    /// first does should normalize at construction, not query, time.
    fn insert(&mut self, domain: &str, action: FilterAction) {
        let labels: Vec<&str> = domain.split('.').collect();
        let mut node = &mut self.root;
        // store labels TLD-first so a single path covers all subdomains
        for label in labels.iter().rev() {
            node = node.children.entry(label.to_string()).or_default();
        }
        node.action = Some(action);
    }

    pub fn lookup(&self, domain: &str) -> FilterAction {
        self.rule_for(domain).unwrap_or(FilterAction::Proxy)
    }

    /// Like [`lookup`](Self::lookup), but reports whether the domain matched an *explicit* rule at
    /// all: `Some(action)` for the deepest explicit rule on the domain's label path, `None` for a
    /// miss. The miss case is what lets the precedence resolver distinguish an explicit `Proxy`
    /// rule (which beats the FST blocklist at level 2) from the trie's default `Proxy` (which does
    /// not) — `lookup` alone cannot tell the two apart.
    pub fn rule_for(&self, domain: &str) -> Option<FilterAction> {
        let labels: Vec<&str> = domain.split('.').collect();
        let mut node = &self.root;
        let mut last_match: Option<&FilterAction> = None;

        for label in labels.iter().rev() {
            match node.children.get(*label) {
                Some(child) => {
                    if let Some(a) = &child.action {
                        last_match = Some(a);
                    }
                    node = child;
                }
                None => break,
            }
        }

        last_match.cloned()
    }
}

// ---------------------------------------------------------------------------
// IpFilter — sorted vec of (prefix_as_u128, mask_as_u128, action) for both
//             IPv4 and IPv6.
//
// Entries are sorted by prefix length descending (most-specific first) so a
// linear scan returns the longest-prefix match on the first hit.  O(n) is
// acceptable for Phase 1 rule counts; a Patricia trie can replace this later.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CidrEntry {
    prefix: u128,
    mask: u128,
    action: FilterAction,
    prefix_len: u8,
}

#[derive(Debug, Default)]
pub struct IpFilter {
    entries: Vec<CidrEntry>,
}

impl IpFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> anyhow::Result<Self> {
        let mut entries = Vec::with_capacity(rules.len());
        for (cidr, action) in rules {
            let entry = parse_cidr(cidr, action.clone())?;
            entries.push(entry);
        }
        // sort by prefix length descending so longest-prefix wins on linear scan
        entries.sort_by(|a, b| b.prefix_len.cmp(&a.prefix_len));
        Ok(IpFilter { entries })
    }

    pub fn lookup(&self, addr: IpAddr) -> FilterAction {
        let bits = ip_to_u128(addr);
        for entry in &self.entries {
            if bits & entry.mask == entry.prefix {
                return entry.action.clone();
            }
        }
        FilterAction::Proxy
    }
}

fn ip_to_u128(addr: IpAddr) -> u128 {
    match addr {
        IpAddr::V4(v4) => {
            // map IPv4 into the IPv4-mapped IPv6 space so both fit in u128
            let octets = v4.octets();
            let v: u32 = u32::from_be_bytes(octets);
            0xffff_0000_0000u128 | (v as u128)
        }
        IpAddr::V6(v6) => u128::from_be_bytes(v6.octets()),
    }
}

fn parse_cidr(cidr: &str, action: FilterAction) -> anyhow::Result<CidrEntry> {
    if let Some((addr_str, len_str)) = cidr.split_once('/') {
        let prefix_len: u8 = len_str.parse()?;
        let addr: IpAddr = addr_str.parse()?;
        let (bits, max_len) = match addr {
            IpAddr::V4(_) => (ip_to_u128(addr), 32u8),
            IpAddr::V6(_) => (ip_to_u128(addr), 128u8),
        };
        anyhow::ensure!(prefix_len <= max_len, "prefix length out of range");
        // for IPv4-mapped, shift the mask to align with the u128 representation
        let shift = match addr {
            IpAddr::V4(_) => 128 - 32,
            IpAddr::V6(_) => 0,
        };
        let mask: u128 = if prefix_len == 0 {
            0
        } else {
            !0u128 << (128 - (prefix_len as u32 + shift as u32))
        };
        let prefix = bits & mask;
        Ok(CidrEntry { prefix, mask, action, prefix_len })
    } else {
        // bare IP address — treat as /32 or /128
        let addr: IpAddr = cidr.parse()?;
        let host_cidr = match addr {
            IpAddr::V4(_) => format!("{}/32", cidr),
            IpAddr::V6(_) => format!("{}/128", cidr),
        };
        parse_cidr(&host_cidr, action)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    // --- DomainFilter ---

    #[test]
    fn domain_exact_match_block() {
        let f = DomainFilter::from_rules(&[("ads.example.com", FilterAction::Block)]);
        assert_eq!(f.lookup("ads.example.com"), FilterAction::Block);
    }

    #[test]
    fn domain_subdomain_inherits_rule() {
        let f = DomainFilter::from_rules(&[("evil.com", FilterAction::Block)]);
        // sub.evil.com should inherit the block on evil.com
        assert_eq!(f.lookup("sub.evil.com"), FilterAction::Block);
        assert_eq!(f.lookup("deep.sub.evil.com"), FilterAction::Block);
    }

    #[test]
    fn domain_no_match_returns_proxy() {
        let f = DomainFilter::from_rules(&[("ads.example.com", FilterAction::Block)]);
        assert_eq!(f.lookup("safe.example.com"), FilterAction::Proxy);
        assert_eq!(f.lookup("unknown.org"), FilterAction::Proxy);
    }

    #[test]
    fn domain_allow_rule() {
        let f = DomainFilter::from_rules(&[
            ("example.com", FilterAction::Block),
            ("safe.example.com", FilterAction::Allow),
        ]);
        assert_eq!(f.lookup("example.com"), FilterAction::Block);
        assert_eq!(f.lookup("safe.example.com"), FilterAction::Allow);
        // other subdomains still inherit the block
        assert_eq!(f.lookup("evil.example.com"), FilterAction::Block);
    }

    #[test]
    fn domain_more_specific_rule_wins() {
        let f = DomainFilter::from_rules(&[
            ("example.com", FilterAction::Block),
            ("cdn.example.com", FilterAction::Allow),
        ]);
        assert_eq!(f.lookup("cdn.example.com"), FilterAction::Allow);
        assert_eq!(f.lookup("other.example.com"), FilterAction::Block);
    }

    #[test]
    fn domain_empty_ruleset_returns_proxy() {
        let f = DomainFilter::from_rules(&[]);
        assert_eq!(f.lookup("anything.com"), FilterAction::Proxy);
    }

    #[test]
    fn rule_for_misses_at_a_branch_node_with_no_action_of_its_own() {
        // Only "cdn.example.com" carries a rule. "example.com" is a real node in the trie (an
        // ancestor on the path) but has no action of its own, so rule_for must report a miss
        // (None) for it, not Some(Proxy) — the precedence resolver depends on this distinction to
        // fall through to the FST blocklist rather than treating "example.com" as an explicit rule.
        let f = DomainFilter::from_rules(&[("cdn.example.com", FilterAction::Block)]);
        assert_eq!(f.rule_for("example.com"), None);
        assert_eq!(f.rule_for("www.example.com"), None);
        // The node with its own action still reports it.
        assert_eq!(f.rule_for("cdn.example.com"), Some(FilterAction::Block));
        // lookup() still collapses the miss to the default, unchanged.
        assert_eq!(f.lookup("example.com"), FilterAction::Proxy);
    }

    #[test]
    fn domain_tld_only_rule() {
        let f = DomainFilter::from_rules(&[("adult", FilterAction::Block)]);
        assert_eq!(f.lookup("adult"), FilterAction::Block);
        assert_eq!(f.lookup("site.adult"), FilterAction::Block);
    }

    // --- IpFilter ---

    #[test]
    fn ip_exact_ipv4_block() {
        let f = IpFilter::from_rules(&[("1.2.3.4/32", FilterAction::Block)]).unwrap();
        assert_eq!(f.lookup("1.2.3.4".parse().unwrap()), FilterAction::Block);
        assert_eq!(f.lookup("1.2.3.5".parse().unwrap()), FilterAction::Proxy);
    }

    #[test]
    fn ip_cidr_range_block() {
        let f = IpFilter::from_rules(&[("10.0.0.0/8", FilterAction::Block)]).unwrap();
        assert_eq!(f.lookup("10.1.2.3".parse().unwrap()), FilterAction::Block);
        assert_eq!(f.lookup("10.255.255.255".parse().unwrap()), FilterAction::Block);
        assert_eq!(f.lookup("11.0.0.0".parse().unwrap()), FilterAction::Proxy);
    }

    #[test]
    fn ip_longest_prefix_wins() {
        let f = IpFilter::from_rules(&[
            ("192.168.0.0/16", FilterAction::Block),
            ("192.168.1.0/24", FilterAction::Allow),
        ])
        .unwrap();
        assert_eq!(f.lookup("192.168.1.5".parse().unwrap()), FilterAction::Allow);
        assert_eq!(f.lookup("192.168.2.5".parse().unwrap()), FilterAction::Block);
    }

    #[test]
    fn ip_no_match_returns_proxy() {
        let f = IpFilter::from_rules(&[("5.5.5.0/24", FilterAction::Block)]).unwrap();
        assert_eq!(f.lookup("8.8.8.8".parse().unwrap()), FilterAction::Proxy);
    }

    #[test]
    fn ip_empty_ruleset_returns_proxy() {
        let f = IpFilter::from_rules(&[]).unwrap();
        assert_eq!(
            f.lookup(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            FilterAction::Proxy
        );
    }

    #[test]
    fn ip_ipv6_block() {
        let f =
            IpFilter::from_rules(&[("2001:db8::/32", FilterAction::Block)]).unwrap();
        let addr: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(f.lookup(addr), FilterAction::Block);
        let outside: IpAddr = "2001:db9::1".parse().unwrap();
        assert_eq!(f.lookup(outside), FilterAction::Proxy);
    }

    #[test]
    fn ip_invalid_cidr_returns_error() {
        let result = IpFilter::from_rules(&[("not-an-ip/24", FilterAction::Block)]);
        assert!(result.is_err());
    }

    #[test]
    fn ip_bare_address_treated_as_host_route() {
        let f = IpFilter::from_rules(&[("9.9.9.9", FilterAction::Block)]).unwrap();
        assert_eq!(f.lookup("9.9.9.9".parse().unwrap()), FilterAction::Block);
        assert_eq!(f.lookup("9.9.9.8".parse().unwrap()), FilterAction::Proxy);
    }
}
