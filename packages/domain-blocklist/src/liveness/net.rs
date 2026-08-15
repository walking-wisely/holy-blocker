//! The real network [`DnsLookup`] implementation for `cli` (module 7) — a message-level DNS
//! client over `hickory-proto`, per the plan's `### 7. cli` "Net-client assumption audit" (run
//! 2026-08-15, against the shipped `hickory-proto` 0.26.1 crate source rather than its docs.rs
//! page). Feature-gated behind `net` so `cargo test` stays fully offline by default — the pure
//! decision logic in [`super::lookup`], [`super::cache`], [`super::canary`] and
//! [`super::corroboration`] needs no live network to test, and this module's own live behavior is
//! exercised only by `#[ignore]`d smoke tests a caller opts into explicitly.
//!
//! [`DnsLookup::lookup`]'s implementation contract (see that trait's doc comment) requires three
//! things a high-level `getaddrinfo`-style resolver API cannot supply, so this client is written
//! directly against `hickory-proto`'s message types (`Message`, `Query`, `Edns`) rather than its
//! `hickory-resolver` convenience crate:
//!
//! 1. Set the DNSSEC OK (DO) bit on every outgoing query — [RFC
//!    3225](https://www.rfc-editor.org/rfc/rfc3225).
//! 2. Read the AD bit and any RFC 8914 Extended DNS Error option off the response —
//!    [RFC 4035 §3.2.3](https://www.rfc-editor.org/rfc/rfc4035#section-3.2.3),
//!    [RFC 8914](https://www.rfc-editor.org/rfc/rfc8914).
//! 3. Retry a truncated (TC=1) UDP response over TCP before giving up — [RFC 1035
//!    §4.2.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.2.1),
//!    [RFC 7766](https://www.rfc-editor.org/rfc/rfc7766).
//!
//! # Reference documents
//!
//! - [RFC 1035 §4.1.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.1.1) — RCODEs this
//!   module's [`interpret_response`] maps onto [`LookupResult`]/[`UnknownReason`].
//! - [RFC 1035 §4.2.2](https://www.rfc-editor.org/rfc/rfc1035#section-4.2.2) — the two-octet
//!   message-length prefix a DNS-over-TCP stream uses, which [`query_tcp`] frames with.
//! - [RFC 6891](https://www.rfc-editor.org/rfc/rfc6891) — EDNS(0), the OPT pseudo-RR the DO bit
//!   and the advertised UDP payload size live on.
//! - [RFC 6604 §3](https://www.rfc-editor.org/rfc/rfc6604#section-3) / [RFC
//!   6672](https://www.rfc-editor.org/rfc/rfc6672) — a CNAME/DNAME chain's terminal RCODE
//!   describes the chain's last name, not the queried one; see [`interpret_response`]'s chain-walk.
//!   Per the plan's net-client audit, this crate's `RData` has **no `DNAME` variant** — a resolver
//!   that performs DNAME substitution surfaces it as a synthesized CNAME record in the answer
//!   section (RFC 6672 §3.4), so chain detection here keys on `RData::CNAME` alone.
//! - [RFC 8914 §4](https://www.rfc-editor.org/rfc/rfc8914#section-4) — the Extended DNS Error
//!   INFO-CODE registry [`decode_ede`] hand-decodes from `EdnsOption::Unknown(15, _)`, since this
//!   crate version has no native EDE decode (per the plan's audit).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use hickory_proto::op::{Edns, Message, Query, ResponseCode};
use hickory_proto::rr::rdata::opt::{EdnsCode, EdnsOption};
use hickory_proto::rr::{Name, RData, Record, RecordType as HickoryRecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use super::lookup::{DnsLookup, LookupResult, RecordType, UnknownReason};

/// The EDNS UDP payload size this client advertises via the OPT record — [RFC 6891
/// §6.2.3](https://www.rfc-editor.org/rfc/rfc6891#section-6.2.3) recommends at least 1220 for
/// IPv6/fragmentation safety; 4096 matches the buffer this client actually reads into and is the
/// commonly-deployed value (`dig`'s own default).
const EDNS_UDP_PAYLOAD: u16 = 4096;
/// UDP receive buffer — must be at least [`EDNS_UDP_PAYLOAD`], since a resolver honoring our
/// advertised payload size may fill it.
const UDP_RECV_BUF: usize = 4096;
/// RFC 1035 §4.2.2's two-octet message-length prefix on a DNS-over-TCP stream.
const TCP_LENGTH_PREFIX_BYTES: usize = 2;
/// Bound on how many CNAME hops [`interpret_response`] will follow within one response before
/// concluding the chain is a loop or absurdly long — see [`DnsLookup`]'s "MUST detect a chain loop
/// and MUST bound how many indirections it will follow" contract. A real chain is rarely more than
/// two or three hops; this is a generous bound with headroom, not a measured deployment maximum.
const MAX_CHAIN_HOPS: usize = 16;
/// Total attempts the UDP leg of one query gets — 1 initial send plus 2 retries, spaced by
/// [`UDP_RETRY_BACKOFF`]. A starting value, not a measured one.
const UDP_RETRY_ATTEMPTS: usize = 3;
/// Fixed delay between UDP retry attempts. Also a starting value, not a measured one. NOTE: this
/// does *not* keep retries inside a single query's `ResolverConfig::timeout` budget by itself —
/// each of [`UDP_RETRY_ATTEMPTS`]'s attempts and the TCP fallback time out independently against
/// that same per-socket-operation budget, so worst case one logical query can take roughly
/// `UDP_RETRY_ATTEMPTS` times the per-attempt timeout plus this backoff between them, plus the TCP
/// fallback's own several timed steps — see [`TOTAL_QUERY_BUDGET`], which is the actual bound on
/// one logical lookup.
const UDP_RETRY_BACKOFF: Duration = Duration::from_millis(100);
/// The one deadline that actually bounds a whole logical lookup ([`query_with_tcp_fallback`]):
/// [`UDP_RETRY_ATTEMPTS`] UDP attempts (each up to 3 timed socket steps) plus up to
/// [`UDP_RETRY_ATTEMPTS`] - 1 backoffs, plus the TCP fallback's 4 timed steps, all against a
/// `ResolverConfig::timeout` that applies per-step rather than per-query. 15 seconds is a starting
/// value with headroom over the qps-paced sweep's expected per-query cost, not a measured one —
/// the sweep's own concurrency budget is sized against this constant, not against
/// `ResolverConfig::timeout` directly, since that field alone understates one query's worst case.
const TOTAL_QUERY_BUDGET: Duration = Duration::from_secs(15);

/// One resolver's UDP/TCP address (RFC 1035 §4.2 transports) and the per-query timeout budget.
/// The plan's default is Cloudflare's unfiltered `1.1.1.1`/`2606:4700:4700::1111` — **never** the
/// filtering `1.1.1.2`/`1.1.1.3` variants, and never the host's system resolver (a CI provider's
/// default is frequently a filtering one, silently and totally).
#[derive(Debug, Clone, Copy)]
pub struct ResolverConfig {
    pub addr: SocketAddr,
    pub timeout: Duration,
}

/// A real, message-level [`DnsLookup`] against one configured resolver over `hickory-proto`.
///
/// Carries **no Tokio runtime of its own**. The async core ([`HickoryDnsLookup::lookup_async`],
/// what the qps-paced sweep driver in `cli` (module 7) calls directly from its own runtime) needs
/// none, and a stored one would panic the moment this value dropped from inside an async task
/// (tokio: "Cannot drop a runtime in a context where blocking is not allowed"). The synchronous
/// `lookup(&self, ...)` trait method instead builds a small current-thread runtime *inside that one
/// function call*, `block_on`s [`HickoryDnsLookup::lookup_async`] on it, and lets it drop there —
/// created and destroyed within a single plain synchronous call, never nested inside an already-
/// running async context, so it can never hit that panic. The same per-call sync→async bridge
/// shape `fetchers::ut1` uses for [`crate::sources::SourceFetcher`], and for the identical reason:
/// the trait this crate's pure `check`/`canary_check`/`corroborate` logic is built against is
/// deliberately synchronous, so it can run with no runtime in tests.
pub struct HickoryDnsLookup {
    config: ResolverConfig,
}

impl HickoryDnsLookup {
    /// Infallible once the runtime field was removed (bug 1); the `Result` return is kept purely
    /// for compatibility with existing `?`-using callers (`sweep.rs`'s `run_sweep`), which are
    /// outside this file and require no edits. The per-call runtime is built where it is needed,
    /// inside [`DnsLookup::lookup`].
    pub fn new(config: ResolverConfig) -> std::io::Result<Self> {
        Ok(Self { config })
    }

    /// The async core `lookup_async` performs: build one query, set the DO bit, send it, retry
    /// over TCP if the UDP answer was truncated, and interpret the response. Never panics or
    /// returns an `Err` — every failure mode this client can observe (malformed input domain,
    /// transport failure, timeout, garbled response) becomes a [`LookupResult::Unknown`] variant,
    /// per this trait's "no I/O error type in the return" contract.
    pub async fn lookup_async(&self, domain: &str, record: RecordType) -> LookupResult {
        let Ok(name) = Name::from_ascii(domain) else {
            // A domain this client can't even parse into wire labels — module 0's normalize()
            // should never hand this client something malformed, but a defensive Unknown here is
            // strictly safer than a panic.
            return LookupResult::Unknown(UnknownReason::Malformed);
        };
        let qtype = to_hickory_record_type(record);

        match query_with_tcp_fallback(self.config, &name, qtype).await {
            Ok(response) => interpret_response(&name, qtype, &response),
            Err(QueryFailure::Timeout) => LookupResult::Unknown(UnknownReason::Timeout),
            Err(QueryFailure::Transport(detail)) => {
                tracing::debug!(domain, ?record, detail, "dns transport failure");
                LookupResult::Unknown(UnknownReason::Transport)
            }
            Err(QueryFailure::Decode(detail)) => {
                tracing::debug!(domain, ?record, detail, "dns response failed to decode");
                LookupResult::Unknown(UnknownReason::Malformed)
            }
            Err(QueryFailure::Mismatch(detail)) => {
                tracing::debug!(domain, ?record, detail, "dns response did not echo the request");
                LookupResult::Unknown(UnknownReason::Malformed)
            }
        }
    }
}

impl DnsLookup for HickoryDnsLookup {
    fn lookup(&self, domain: &str, record: RecordType) -> LookupResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build a runtime for HickoryDnsLookup::lookup — this is the sync \
                     trait bridge, only ever called from plain sync code (tests, or a future \
                     non-async caller), never from inside an already-running async context");
        rt.block_on(self.lookup_async(domain, record))
    }
}

fn to_hickory_record_type(record: RecordType) -> HickoryRecordType {
    match record {
        RecordType::A => HickoryRecordType::A,
        RecordType::Aaaa => HickoryRecordType::AAAA,
    }
}

/// Why a query attempt produced no usable [`Message`]. Never surfaced to a caller directly — see
/// [`HickoryDnsLookup::lookup_async`], which maps every variant onto a [`LookupResult::Unknown`]
/// reason.
#[derive(Debug)]
enum QueryFailure {
    Timeout,
    Transport(String),
    Decode(String),
    /// The response decoded cleanly but is not a reply to the request that produced it — its
    /// header ID or question section does not echo the request (see [`check_response_echo`]).
    /// Treated like [`QueryFailure::Decode`]: not retried, surfaced as "no trustworthy evidence".
    Mismatch(String),
}

/// Builds and sends one query over UDP (with the UDP leg's own retry loop); if the response is
/// truncated (TC=1), retries the *same* query over TCP per [RFC 1035
/// §4.2.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.2.1) /
/// [RFC 7766](https://www.rfc-editor.org/rfc/rfc7766) before returning — satisfying
/// [`DnsLookup`]'s truncation-retry contract rather than surfacing `Malformed` on a truncated
/// answer this client could have completed. Every response that is actually returned passes
/// [`check_response_echo`] first, on both the non-truncated UDP path and the TCP fallback path.
async fn query_with_tcp_fallback(
    config: ResolverConfig,
    name: &Name,
    qtype: HickoryRecordType,
) -> Result<Message, QueryFailure> {
    // One deadline for the whole logical lookup — see TOTAL_QUERY_BUDGET's doc comment for why
    // the per-socket-operation timeouts inside query_udp_with_retry/query_tcp do not add up to one
    // by themselves.
    tokio::time::timeout(TOTAL_QUERY_BUDGET, query_with_tcp_fallback_inner(config, name, qtype))
        .await
        .unwrap_or(Err(QueryFailure::Timeout))
}

async fn query_with_tcp_fallback_inner(
    config: ResolverConfig,
    name: &Name,
    qtype: HickoryRecordType,
) -> Result<Message, QueryFailure> {
    let request = build_query(name, qtype);
    let request_bytes = request
        .to_vec()
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    let response = query_udp_with_retry(config, &request_bytes).await?;
    if !response.metadata.truncation {
        check_response_echo(request.metadata.id, name, qtype, &response)?;
        return Ok(response);
    }
    let response = query_tcp(config, &request_bytes).await?;
    check_response_echo(request.metadata.id, name, qtype, &response)?;
    Ok(response)
}

/// The UDP leg with its retry loop: up to [`UDP_RETRY_ATTEMPTS`] total [`query_udp`] attempts,
/// spaced by [`UDP_RETRY_BACKOFF`]. Loss on a single UDP datagram is normal, not exceptional, so a
/// plausibly-transient failure is retried rather than surrendered to; only
/// [`QueryFailure::Timeout`]/[`QueryFailure::Transport`] qualify (see [`is_retryable_udp_failure`])
/// — a `Decode` (the resolver answered with garbage, which asking the same way again will not fix)
/// or a response-shape `Mismatch` (impossible from [`query_udp`], which does no verification)
/// returns immediately. Only this UDP leg retries; the TCP fallback stays single-attempt, since its
/// TC=1 retry semantics already give it one retry-shaped path.
async fn query_udp_with_retry(
    config: ResolverConfig,
    request_bytes: &[u8],
) -> Result<Message, QueryFailure> {
    let mut last_failure = None;
    for attempt in 0..UDP_RETRY_ATTEMPTS {
        match query_udp(config, request_bytes).await {
            Ok(response) => return Ok(response),
            Err(failure) if is_retryable_udp_failure(&failure) => {
                last_failure = Some(failure);
                if let Some(backoff) = udp_retry_schedule(attempt) {
                    tokio::time::sleep(backoff).await;
                }
            }
            Err(failure) => return Err(failure),
        }
    }
    Err(last_failure.expect(
        "a retryable UDP failure is always recorded before the attempt loop can exhaust",
    ))
}

/// Whether a failed UDP attempt is plausibly transient and worth retrying — a `Timeout` (nothing
/// came back in time) or a `Transport` error (bind/send/recv returned an OS error) can both be the
/// result of a dropped datagram and improve on a second try. A `Decode` failure means the resolver
/// did answer but the bytes were garbage — asking identically again will not fix that — and
/// `Mismatch` cannot originate from [`query_udp`] at all (verification happens upstack).
fn is_retryable_udp_failure(failure: &QueryFailure) -> bool {
    matches!(failure, QueryFailure::Timeout | QueryFailure::Transport(_))
}

/// The backoff to sleep before retrying zero-based `attempt`, or `None` when `attempt` was the
/// last one permitted (i.e. `Some` exactly when another attempt will follow). Extracted so the
/// retry count/backoff logic is unit-testable without real socket I/O.
fn udp_retry_schedule(attempt: usize) -> Option<Duration> {
    if attempt + 1 < UDP_RETRY_ATTEMPTS {
        Some(UDP_RETRY_BACKOFF)
    } else {
        None
    }
}

/// Constructs one query message: a single question for `name`/`qtype`, IN class (the `Query`
/// default), with EDNS(0) attached and the DNSSEC OK bit set — see this module's doc comment for
/// why the DO bit is mandatory here rather than optional.
///
/// `Message::query()` already gives every request a fresh random ID (`hickory-proto` 0.26.1's own
/// `Message::query()` calls `Self::new(random(), ..)` — verified against this crate's own pinned
/// source, not its docs, since a web search surfaced a claim for a hypothetical future release
/// that a bare `Message::query()` leaves the ID at `0`; that claim does not hold for the version
/// this crate actually pins). What `Message::query()` does *not* set is Recursion Desired — its
/// `Header`/`Metadata` default is `recursion_desired: false` — so it is set explicitly here: this
/// client queries public recursive resolvers ([`ResolverConfig`]'s doc comment names
/// `1.1.1.1`/`8.8.8.8`), not authoritative servers, and an RD-unset query is a request for
/// iterative-only (referral) behavior a recursive resolver is not obligated to answer the way this
/// client's response interpretation assumes — [RFC 1035
/// §4.1.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.1.1).
fn build_query(name: &Name, qtype: HickoryRecordType) -> Message {
    let mut message = Message::query();
    message.metadata.recursion_desired = true;
    let mut query = Query::new();
    query.set_name(name.clone());
    query.set_query_type(qtype);
    message.add_query(query);

    let mut edns = Edns::new();
    edns.set_dnssec_ok(true);
    edns.set_max_payload(EDNS_UDP_PAYLOAD);
    message.set_edns(edns);

    message
}

/// One UDP round trip. Uses a connected socket (`UdpSocket::connect`) so the kernel filters
/// incoming datagrams to the configured resolver's address — a real (if partial) defense against
/// off-path UDP spoofing. Genuine request-ID/question-echo verification happens upstack once a
/// response has decoded: [`check_response_echo`], called from [`query_with_tcp_fallback`] on the
/// response that will actually be returned.
async fn query_udp(config: ResolverConfig, request_bytes: &[u8]) -> Result<Message, QueryFailure> {
    let bind_addr: SocketAddr = if config.addr.is_ipv6() {
        "[::]:0".parse().expect("valid literal socket address")
    } else {
        "0.0.0.0:0".parse().expect("valid literal socket address")
    };

    let socket = tokio::time::timeout(config.timeout, async {
        let socket = UdpSocket::bind(bind_addr).await?;
        socket.connect(config.addr).await?;
        Ok::<_, std::io::Error>(socket)
    })
    .await
    .map_err(|_| QueryFailure::Timeout)?
    .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    tokio::time::timeout(config.timeout, socket.send(request_bytes))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    let mut buf = [0u8; UDP_RECV_BUF];
    let n = tokio::time::timeout(config.timeout, socket.recv(&mut buf))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    decode_response(&buf[..n])
}

/// One TCP round trip, RFC 1035 §4.2.2-framed: a two-octet big-endian length prefix, then exactly
/// that many message bytes, in both directions.
async fn query_tcp(config: ResolverConfig, request_bytes: &[u8]) -> Result<Message, QueryFailure> {
    let mut stream = tokio::time::timeout(config.timeout, TcpStream::connect(config.addr))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    let mut framed = Vec::with_capacity(TCP_LENGTH_PREFIX_BYTES + request_bytes.len());
    framed.extend_from_slice(&(request_bytes.len() as u16).to_be_bytes());
    framed.extend_from_slice(request_bytes);

    tokio::time::timeout(config.timeout, stream.write_all(&framed))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    let mut len_buf = [0u8; TCP_LENGTH_PREFIX_BYTES];
    tokio::time::timeout(config.timeout, stream.read_exact(&mut len_buf))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;
    let response_len = u16::from_be_bytes(len_buf) as usize;

    let mut response_buf = vec![0u8; response_len];
    tokio::time::timeout(config.timeout, stream.read_exact(&mut response_buf))
        .await
        .map_err(|_| QueryFailure::Timeout)?
        .map_err(|e| QueryFailure::Transport(e.to_string()))?;

    decode_response(&response_buf)
}

fn decode_response(bytes: &[u8]) -> Result<Message, QueryFailure> {
    Message::from_vec(bytes).map_err(|e| QueryFailure::Decode(e.to_string()))
}

/// Verifies that a decoded response is actually a reply to the request that produced it: its
/// header ID must equal the request's ID (`Message::query()` gives every request a fresh random
/// ID — see [`build_query`]), and its question section (RFC 1035 §4.1.2, "the question section ...
/// is carried in the response") must echo back the name and type that were actually queried. The
/// ID is the RFC 1035 §4.1.1 "identifier ... copied to the corresponding reply"; a connected
/// socket already filters the source address, and this closes the remaining gap. A response that
/// fails any check is not trustworthy evidence *about this query*, no matter how well-formed it is
/// — same evidentiary status as an unparseable one — so a mismatch is reported as
/// [`QueryFailure::Mismatch`] rather than returned as `Ok(response)`.
fn check_response_echo(
    request_id: u16,
    name: &Name,
    qtype: HickoryRecordType,
    response: &Message,
) -> Result<(), QueryFailure> {
    if response.metadata.id != request_id {
        return Err(QueryFailure::Mismatch(format!(
            "response id {} does not match request id {}",
            response.metadata.id, request_id
        )));
    }
    let echo = response.queries.first().ok_or_else(|| {
        QueryFailure::Mismatch("response has no question entry to echo back".to_string())
    })?;
    if echo.name() != name {
        return Err(QueryFailure::Mismatch(format!(
            "response echoes question name {} but {} was queried",
            echo.name(),
            name
        )));
    }
    if echo.query_type() != qtype {
        return Err(QueryFailure::Mismatch(format!(
            "response echoes question type {} but {} was queried",
            echo.query_type(),
            qtype
        )));
    }
    Ok(())
}

/// Interprets a decoded response for the query that produced it — the pure heart of this module,
/// deliberately free of any I/O so it can be unit-tested against hand-built [`Message`] fixtures
/// without a real resolver.
///
/// Walks the answer section from `query_name`, following CNAME hops (RFC 1035 §3.3.1; a DNAME hop
/// surfaces as a synthesized CNAME in this crate — see the module doc comment) up to
/// [`MAX_CHAIN_HOPS`] times, with a visited-name set so a hostile or buggy resolver's `a -> a` (or
/// longer) cycle is caught rather than looped over. At each name in the chain:
///
/// - A matching-type (`qtype`) record present is [`LookupResult::Resolved`] with every such
///   record's address, regardless of how many CNAME hops preceded it.
/// - Otherwise, a CNAME record at that name continues the walk one hop further.
/// - Otherwise, the walk stops at that name and the response's own RCODE decides the outcome —
///   `NXDomain` on the *queried* name (no hops followed) is a direct [`LookupResult::NxDomain`];
///   the same RCODE after at least one hop is [`LookupResult::NxDomainViaChain`], per RFC 6604 §3 /
///   RFC 6672 — the RCODE describes the chain's last name, not the one actually queried.
fn interpret_response(
    query_name: &Name,
    qtype: HickoryRecordType,
    response: &Message,
) -> LookupResult {
    let mut current = query_name.clone();
    let mut visited: HashSet<Name> = HashSet::new();
    let mut followed_chain = false;

    for _ in 0..MAX_CHAIN_HOPS {
        if !visited.insert(current.clone()) {
            // A CNAME cycle (or a chain revisiting an earlier name) — never trustworthy evidence
            // either way, per DnsLookup's loop-detection contract.
            return LookupResult::Unknown(UnknownReason::Malformed);
        }

        let addrs = addresses_at(&response.answers, &current, qtype);
        if !addrs.is_empty() {
            return LookupResult::Resolved(addrs);
        }

        match cname_target_at(&response.answers, &current) {
            Some(target) => {
                followed_chain = true;
                current = target;
            }
            None => break,
        }
    }

    match response.metadata.response_code {
        ResponseCode::NoError => LookupResult::Unknown(UnknownReason::NoData),
        ResponseCode::NXDomain => {
            if followed_chain {
                LookupResult::NxDomainViaChain
            } else {
                LookupResult::NxDomain {
                    authenticated: response.metadata.authentic_data,
                    extended_error: decode_ede(response.edns.as_ref()),
                }
            }
        }
        // RFC 6672 §2.2 YXDOMAIN: a DNAME substitution would exceed 255 octets. Evidence about
        // the chain's target, never proof the queried name itself is unregistered — the same
        // evidentiary status as an ordinary chain-to-dead-target NXDOMAIN.
        ResponseCode::YXDomain => LookupResult::NxDomainViaChain,
        ResponseCode::ServFail => LookupResult::Unknown(UnknownReason::ServFail),
        ResponseCode::Refused => LookupResult::Unknown(UnknownReason::Refused),
        ResponseCode::FormErr => LookupResult::Unknown(UnknownReason::FormErr),
        ResponseCode::NotImp => LookupResult::Unknown(UnknownReason::NotImp),
        _ => LookupResult::Unknown(UnknownReason::Malformed),
    }
}

/// Every address from a `qtype`-matching record owned by `name` in `answers`.
fn addresses_at(
    answers: &[Record],
    name: &Name,
    qtype: HickoryRecordType,
) -> Vec<std::net::IpAddr> {
    answers
        .iter()
        .filter(|record| record.name == *name && record.record_type() == qtype)
        .filter_map(|record| match &record.data {
            RData::A(a) => Some(std::net::IpAddr::V4(a.0)),
            RData::AAAA(aaaa) => Some(std::net::IpAddr::V6(aaaa.0)),
            _ => None,
        })
        .collect()
}

/// The first CNAME record's target owned by `name`, if any — the next hop in the chain walk.
fn cname_target_at(answers: &[Record], name: &Name) -> Option<Name> {
    answers.iter().find_map(|record| {
        if record.name != *name || record.record_type() != HickoryRecordType::CNAME {
            return None;
        }
        match &record.data {
            RData::CNAME(cname) => Some(cname.0.clone()),
            _ => None,
        }
    })
}

/// Hand-decodes an RFC 8914 §4 Extended DNS Error option from `edns`'s OPT record, if present.
/// `hickory-proto` 0.26.1 has no native EDE type (per the plan's net-client audit) — an EDE lands
/// in `EdnsOption::Unknown(15, raw)`, whose first two octets (big-endian) are the INFO-CODE per
/// [RFC 8914 §3.2](https://www.rfc-editor.org/rfc/rfc8914#section-3.2); any EXTRA-TEXT bytes after
/// that are informational only and unused here.
fn decode_ede(edns: Option<&Edns>) -> Option<u16> {
    let edns = edns?;
    match edns.option(EdnsCode::Unknown(15))? {
        EdnsOption::Unknown(15, raw) if raw.len() >= 2 => {
            Some(u16::from_be_bytes([raw[0], raw[1]]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::{MessageType, OpCode};
    use hickory_proto::rr::RData;
    use hickory_proto::rr::rdata::{A, AAAA, CNAME};
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    fn name(s: &str) -> Name {
        Name::from_ascii(s).unwrap()
    }

    fn base_response(rcode: ResponseCode) -> Message {
        let mut message = Message::new(1, MessageType::Response, OpCode::Query);
        message.metadata.response_code = rcode;
        message
    }

    fn a_record(owner: &str, addr: Ipv4Addr) -> Record {
        Record::from_rdata(name(owner), 300, RData::A(A(addr)))
    }

    fn aaaa_record(owner: &str, addr: Ipv6Addr) -> Record {
        Record::from_rdata(name(owner), 300, RData::AAAA(AAAA(addr)))
    }

    fn cname_record(owner: &str, target: &str) -> Record {
        Record::from_rdata(name(owner), 300, RData::CNAME(CNAME(name(target))))
    }

    fn echo_query(name_in: &str, qtype: HickoryRecordType) -> Query {
        let mut query = Query::new();
        query.set_name(name(name_in));
        query.set_query_type(qtype);
        query
    }

    #[test]
    fn a_direct_answer_is_resolved() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_answer(a_record("example.com.", Ipv4Addr::new(192, 0, 2, 1)));
        let result = interpret_response(&name("example.com."), HickoryRecordType::A, &response);
        assert_eq!(
            result,
            LookupResult::Resolved(vec![std::net::IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))])
        );
    }

    #[test]
    fn an_aaaa_answer_is_resolved() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_answer(aaaa_record("example.com.", Ipv6Addr::LOCALHOST));
        let result = interpret_response(&name("example.com."), HickoryRecordType::AAAA, &response);
        assert_eq!(
            result,
            LookupResult::Resolved(vec![std::net::IpAddr::V6(Ipv6Addr::LOCALHOST)])
        );
    }

    #[test]
    fn a_direct_nxdomain_carries_authentication_and_ede_evidence() {
        let mut response = base_response(ResponseCode::NXDomain);
        response.metadata.authentic_data = true;
        let result = interpret_response(&name("dead.example."), HickoryRecordType::A, &response);
        assert_eq!(
            result,
            LookupResult::NxDomain {
                authenticated: true,
                extended_error: None,
            }
        );
    }

    #[test]
    fn nodata_is_unknown_not_dead() {
        let response = base_response(ResponseCode::NoError);
        let result = interpret_response(&name("apex.example."), HickoryRecordType::A, &response);
        assert_eq!(result, LookupResult::Unknown(UnknownReason::NoData));
    }

    #[test]
    fn a_cname_chain_that_resolves_is_still_resolved() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_answer(cname_record("www.example.com.", "edge.example.net."));
        response.add_answer(a_record("edge.example.net.", Ipv4Addr::new(198, 51, 100, 7)));
        let result = interpret_response(
            &name("www.example.com."),
            HickoryRecordType::A,
            &response,
        );
        assert_eq!(
            result,
            LookupResult::Resolved(vec![std::net::IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))])
        );
    }

    #[test]
    fn nxdomain_reached_via_a_cname_chain_is_via_chain_not_direct() {
        // RFC 6604 §3 / RFC 6672: the RCODE describes the chain's last name, not the queried one.
        let mut response = base_response(ResponseCode::NXDomain);
        response.add_answer(cname_record("www.example.com.", "dead-target.example."));
        let result = interpret_response(
            &name("www.example.com."),
            HickoryRecordType::A,
            &response,
        );
        assert_eq!(result, LookupResult::NxDomainViaChain);
    }

    #[test]
    fn a_cname_self_loop_is_malformed_not_an_infinite_walk() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_answer(cname_record("loop.example.", "loop.example."));
        let result = interpret_response(&name("loop.example."), HickoryRecordType::A, &response);
        assert_eq!(result, LookupResult::Unknown(UnknownReason::Malformed));
    }

    #[test]
    fn servfail_refused_formerr_notimp_map_to_their_own_unknown_reasons() {
        for (rcode, reason) in [
            (ResponseCode::ServFail, UnknownReason::ServFail),
            (ResponseCode::Refused, UnknownReason::Refused),
            (ResponseCode::FormErr, UnknownReason::FormErr),
            (ResponseCode::NotImp, UnknownReason::NotImp),
        ] {
            let response = base_response(rcode);
            let result = interpret_response(&name("example.com."), HickoryRecordType::A, &response);
            assert_eq!(result, LookupResult::Unknown(reason));
        }
    }

    #[test]
    fn decode_ede_reads_the_info_code_from_a_synthetic_option() {
        let mut edns = Edns::new();
        edns.options_mut().insert(EdnsOption::Unknown(15, vec![0x00, 0x13]));
        // 0x0013 = 19 = RFC 8914 Stale NXDOMAIN Answer.
        assert_eq!(decode_ede(Some(&edns)), Some(19));
    }

    #[test]
    fn decode_ede_is_none_with_no_edns_or_no_ede_option() {
        assert_eq!(decode_ede(None), None);
        assert_eq!(decode_ede(Some(&Edns::new())), None);
    }

    #[test]
    fn build_query_sets_the_dnssec_ok_bit() {
        let message = build_query(&name("example.com."), HickoryRecordType::A);
        let edns = message.edns.expect("build_query always attaches EDNS");
        assert!(edns.flags().dnssec_ok);
    }

    #[test]
    fn build_query_sets_recursion_desired() {
        let message = build_query(&name("example.com."), HickoryRecordType::A);
        assert!(message.metadata.recursion_desired);
    }

    #[test]
    fn build_query_ids_are_not_constant_across_calls() {
        // `Message::query()` already randomizes the ID (verified against this crate's own pinned
        // source — see build_query's doc comment); this is a smoke check that still holds, not a
        // re-derivation of hickory-proto's own randomness.
        let ids: std::collections::HashSet<u16> = (0..16)
            .map(|_| build_query(&name("example.com."), HickoryRecordType::A).metadata.id)
            .collect();
        assert!(ids.len() > 1, "16 fresh queries produced only one distinct id: {ids:?}");
    }

    #[test]
    fn from_str_smoke_for_names_used_in_fixtures() {
        // A cheap sanity check that this module's Name literals parse the way the tests assume.
        assert_eq!(Name::from_str("example.com.").unwrap(), name("example.com."));
    }

    #[test]
    fn a_response_echoing_id_and_question_is_accepted() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_query(echo_query("example.com.", HickoryRecordType::A));
        assert!(matches!(
            check_response_echo(1, &name("example.com."), HickoryRecordType::A, &response),
            Ok(())
        ));
    }

    #[test]
    fn a_response_with_the_wrong_id_is_rejected() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_query(echo_query("example.com.", HickoryRecordType::A));
        let err = check_response_echo(2, &name("example.com."), HickoryRecordType::A, &response)
            .expect_err("a mismatched id must be rejected");
        assert!(matches!(err, QueryFailure::Mismatch(_)));
    }

    #[test]
    fn a_response_answering_a_different_question_name_is_rejected() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_query(echo_query("other.example.", HickoryRecordType::A));
        let err = check_response_echo(1, &name("example.com."), HickoryRecordType::A, &response)
            .expect_err("a question-name mismatch must be rejected");
        assert!(matches!(err, QueryFailure::Mismatch(_)));
    }

    #[test]
    fn a_response_answering_a_different_question_type_is_rejected() {
        let mut response = base_response(ResponseCode::NoError);
        response.add_query(echo_query("example.com.", HickoryRecordType::AAAA));
        let err = check_response_echo(1, &name("example.com."), HickoryRecordType::A, &response)
            .expect_err("a question-type mismatch must be rejected");
        assert!(matches!(err, QueryFailure::Mismatch(_)));
    }

    #[test]
    fn a_response_with_no_question_section_is_rejected() {
        let response = base_response(ResponseCode::NoError);
        let err = check_response_echo(1, &name("example.com."), HickoryRecordType::A, &response)
            .expect_err("an empty question section must be rejected");
        assert!(matches!(err, QueryFailure::Mismatch(_)));
    }

    #[test]
    fn only_timeout_and_transport_failures_are_retryable() {
        assert!(is_retryable_udp_failure(&QueryFailure::Timeout));
        assert!(is_retryable_udp_failure(&QueryFailure::Transport(
            "connection reset".to_string()
        )));
        assert!(!is_retryable_udp_failure(&QueryFailure::Decode("garbage".to_string())));
        assert!(!is_retryable_udp_failure(&QueryFailure::Mismatch("id".to_string())));
    }

    #[test]
    fn udp_retry_schedule_grants_two_retries_then_stops() {
        // UDP_RETRY_ATTEMPTS = 3: attempts 0 and 1 are followed by a backoff sleep, attempt 2 is
        // the last one allowed, and anything past it is belt-and-braces none.
        assert_eq!(udp_retry_schedule(0), Some(UDP_RETRY_BACKOFF));
        assert_eq!(udp_retry_schedule(1), Some(UDP_RETRY_BACKOFF));
        assert_eq!(udp_retry_schedule(UDP_RETRY_ATTEMPTS - 1), None);
        assert_eq!(udp_retry_schedule(UDP_RETRY_ATTEMPTS), None);
    }
}
