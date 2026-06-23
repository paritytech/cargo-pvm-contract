//! `measure-ordered-index` — emits a single-line JSON object describing the
//! on-chain storage cost of an OrderedIndex workload, measured in the real
//! `pallet-revive` weight unit (picoseconds of ref_time + bytes of proof_size)
//! that Polkadot Asset Hub Westend charges — not a synthetic read counter.
//!
//! # JSON contract
//!
//! Prints exactly one line to stdout (a single `\n`-terminated JSON object).
//! All diagnostics go to stderr.
//!
//! ```json
//! {"n":10000,"queries":1000,"t":10,
//!  "weight_ref_time_per_query":1234567,
//!  "weight_proof_size_per_query":89,
//!  "slot_reads_per_query":12.34,
//!  "insert_weight_ref_time":999999,
//!  "insert_writes":1234,"insert_clears":0,
//!  "range_p50_ns":5678,"range_p99_ns":9101,
//!  "correctness":true}
//! ```
//!
//! Primary metric (the optimization target): `weight_ref_time_per_query` — the
//! average `pallet-revive` ref_time weight (picoseconds) per prefix-range
//! query. `weight_proof_size_per_query` is the PoV/state-proof analogue.
//! `slot_reads_per_query` is retained as a diagnostic (the prior metric).
//!
//! # Configuration
//!
//! - `BENCH_N` — number of records inserted (default 10000)
//! - `BENCH_Q` — number of prefix-range queries (default 1000)
//! - `BENCH_T` — OrderedIndex degree T (default 2, range 2..=12)

#![cfg(not(target_arch = "riscv64"))]

use std::collections::BTreeMap;
use std::env;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use core::ops::Bound;

use pvm_contract_types::{Host, HostApi, MockHostBuilder};
use pvm_storage::ordered_index::OrderedIndex;

#[path = "../../benches/counting_host.rs"]
mod counting_host;
use counting_host::CountingHost;

fn parse_env(name: &str, default: u64) -> u64 {
    match env::var(name) {
        Ok(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[bench] {}={:?} is not a valid u64: {}; using default {}",
                    name, s, e, default
                );
                default
            }
        },
        Err(_) => default,
    }
}

fn percentile_index(count: usize, pct: u64) -> usize {
    let pct = pct.min(100);
    ((count as u64).saturating_sub(1) * pct / 100) as usize
}

fn run_for_t<const T: usize>(n: u64, q: u64) -> ExitCode {
    let mock = MockHostBuilder::new().build();
    let counting = CountingHost::new(Rc::new(mock));
    let dyn_host: Rc<dyn HostApi> = counting.clone();
    let host = Host::from_dyn(dyn_host);

    let idx = OrderedIndex::<String, u64, T>::new(b"bench", host.clone());
    let mut oracle: BTreeMap<String, u64> = BTreeMap::new();

    for i in 0..n {
        let k = format!("user{:04}", i);
        idx.insert(&host, &k, &i);
        oracle.insert(k, i);
    }
    let snap_after_insert = counting.snapshot();
    let insert_weight_ref_time = snap_after_insert.ref_time_ps;

    counting.reset();

    let mut latencies_ns: Vec<u64> = Vec::with_capacity(q as usize);
    let mut all_correct = true;
    for q_idx in 0..q {
        let p = format!("user{:03}", q_idx % 1000);
        let upper: String = {
            let mut s = String::with_capacity(p.len() + 1);
            s.push_str(&p);
            s.push('\u{FF}');
            s
        };

        let start = Instant::now();
        let actual = idx.range(&host, Bound::Included(&p), Bound::Excluded(&upper), 0, 100);
        let elapsed_ns = start.elapsed().as_nanos() as u64;
        latencies_ns.push(elapsed_ns);

        let expected: Vec<(String, u64)> = oracle
            .range::<String, _>((Bound::Included(p.clone()), Bound::Excluded(upper.clone())))
            .map(|(k, v)| (k.clone(), *v))
            .take(100)
            .collect();
        if actual.len() != expected.len() || actual != expected {
            eprintln!(
                "[bench] correctness FAIL q={} prefix={} expected={} got={}",
                q_idx,
                p,
                expected.len(),
                actual.len()
            );
            all_correct = false;
        }
    }

    let snap_after_queries = counting.snapshot();
    let total_reads = snap_after_queries.reads;
    let avg_reads_per_query = total_reads as f64 / q as f64;
    let weight_ref_time_per_query = snap_after_queries.ref_time_ps / q;
    let weight_proof_size_per_query = snap_after_queries.proof_size_bytes / q;

    latencies_ns.sort_unstable();
    let p50_idx = percentile_index(latencies_ns.len(), 50);
    let p99_idx = percentile_index(latencies_ns.len(), 99);
    let p50_ns = latencies_ns[p50_idx];
    let p99_ns = latencies_ns[p99_idx];

    eprintln!(
        "[bench] n={} q={} t={} weight_ref_time/q={} weight_proof/q={} reads/q={:.3} \
         insert.weight_ref={} insert.writes={} insert.clears={} p50={}ns p99={}ns correct={}",
        n,
        q,
        T,
        weight_ref_time_per_query,
        weight_proof_size_per_query,
        avg_reads_per_query,
        insert_weight_ref_time,
        snap_after_insert.writes,
        snap_after_insert.clears,
        p50_ns,
        p99_ns,
        all_correct
    );

    println!(
        "{{\"n\":{},\"queries\":{},\"t\":{},\
         \"weight_ref_time_per_query\":{},\"weight_proof_size_per_query\":{},\
         \"slot_reads_per_query\":{:.2},\"insert_weight_ref_time\":{},\
         \"insert_writes\":{},\"insert_clears\":{},\
         \"range_p50_ns\":{},\"range_p99_ns\":{},\"correctness\":{}}}",
        n,
        q,
        T,
        weight_ref_time_per_query,
        weight_proof_size_per_query,
        avg_reads_per_query,
        insert_weight_ref_time,
        snap_after_insert.writes,
        snap_after_insert.clears,
        p50_ns,
        p99_ns,
        all_correct
    );

    if all_correct {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let n = parse_env("BENCH_N", 10_000);
    let q = parse_env("BENCH_Q", 1_000);
    let t = parse_env("BENCH_T", 2);

    match t {
        2 => run_for_t::<2>(n, q),
        3 => run_for_t::<3>(n, q),
        4 => run_for_t::<4>(n, q),
        5 => run_for_t::<5>(n, q),
        6 => run_for_t::<6>(n, q),
        7 => run_for_t::<7>(n, q),
        8 => run_for_t::<8>(n, q),
        9 => run_for_t::<9>(n, q),
        10 => run_for_t::<10>(n, q),
        11 => run_for_t::<11>(n, q),
        12 => run_for_t::<12>(n, q),
        other => {
            eprintln!("[bench] BENCH_T={} not supported (range 2..=12)", other);
            ExitCode::from(2)
        }
    }
}
