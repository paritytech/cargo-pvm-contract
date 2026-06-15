#![cfg(not(target_arch = "riscv64"))]

use std::collections::BTreeMap;
use std::env;
use std::ops::Bound;
use std::rc::Rc;
use std::time::Instant;

use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::ordered_index::OrderedIndex;

#[allow(dead_code)]
#[path = "../../benches/counting_host.rs"]
mod counting_host;
use counting_host::CountingHost;

fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn uniform_key(state: &mut u32) -> String {
    let len = 5 + (xorshift32(state) as usize % 8);
    let mut s = String::with_capacity(len + 3);
    for _ in 0..len {
        s.push(ALNUM[(xorshift32(state) as usize) % ALNUM.len()] as char);
    }
    use core::fmt::Write;
    write!(s, ".{:02}", xorshift32(state) % 100).unwrap();
    s
}

fn zipf_key(state: &mut u32, i: usize) -> String {
    const NAMES: &[&str] = &[
        "alice", "bob", "charlie", "dave", "eve", "frank", "grace", "heidi", "ivan", "judy",
        "karl", "leo", "mallory", "nancy", "oscar", "peggy", "quinn", "ruth", "sam", "trent",
        "victor", "walter", "xavier", "yara", "zara", "oliver", "emma", "liam", "ava", "noah",
        "mia", "lucas", "sofia", "ethan", "isla", "mason", "luna", "logan", "zoe", "max",
    ];
    let r = (xorshift32(state) as f64 / u32::MAX as f64) * (NAMES.len() as f64) * 2.0;
    let idx = (r as usize).min(NAMES.len() - 1);
    format!("{}.{:06}", NAMES[idx], i)
}

fn run_profile<const T: usize>(profile: &str, n: usize, q: usize) {
    let host_inner = Rc::new(MockHostBuilder::new().build());
    let counting = CountingHost::new(host_inner);
    let host = Host::from_dyn(counting.clone());
    let idx = OrderedIndex::<String, u64, T>::new(b"validate", Host::from_dyn(counting.clone()));

    let mut oracle: BTreeMap<String, u64> = BTreeMap::new();
    let mut state: u32 = 42;

    for i in 0..n {
        let key = match profile {
            "uniform" => uniform_key(&mut state),
            "zipf" => zipf_key(&mut state, i),
            _ => format!("user{:06}", i),
        };
        let val = i as u64;
        idx.insert(&host, &key, &val);
        oracle.insert(key, i as u64);
    }

    counting.reset();

    let mut state_q: u32 = 99;
    let mut latencies: Vec<u64> = Vec::with_capacity(q);
    let mut total_reads: u64 = 0;
    let mut correct = true;

    for _ in 0..q {
        let mut p = match profile {
            "uniform" => uniform_key(&mut state_q),
            "zipf" => zipf_key(&mut state_q, 0),
            _ => format!("user{:03}", xorshift32(&mut state_q) % 1000),
        };
        let plen = 1 + (xorshift32(&mut state_q) as usize % 4);
        p.truncate(plen.min(p.len()));
        let upper = format!("{}\u{FF}", p);

        let reads_before = counting.reads();
        let start = Instant::now();
        let results = idx.range(&host, Bound::Included(&p), Bound::Excluded(&upper), 0, 100);
        latencies.push(start.elapsed().as_nanos() as u64);
        total_reads += counting.reads() - reads_before;

        if !correct {
            continue;
        }
        let oracle_results: Vec<(String, u64)> = oracle
            .range((Bound::Included(p.clone()), Bound::Excluded(upper)))
            .take(100)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        if results.len() != oracle_results.len() {
            correct = false;
            continue;
        }
        for (a, b) in results.iter().zip(oracle_results.iter()) {
            if a.0 != b.0 || a.1 != b.1 {
                correct = false;
                break;
            }
        }
    }

    latencies.sort_unstable();
    let p50 = latencies.get(latencies.len() / 2).copied().unwrap_or(0);
    let p99 = latencies
        [((latencies.len() as f64 * 0.99) as usize).min(latencies.len().saturating_sub(1))];

    let avg_reads = (total_reads as f64 / q as f64 * 100.0).round() / 100.0;
    println!(
        r#"{{"profile":"{}","n":{},"queries":{},"slot_reads_per_query":{},"range_p50_ns":{},"range_p99_ns":{},"correctness":{}}}"#,
        profile, n, q, avg_reads, p50, p99, correct
    );
}

fn main() {
    let n: usize = env::var("VAL_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let q: usize = env::var("VAL_Q")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000);
    let t: usize = env::var("VAL_T")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    eprintln!("[validate] n={} q={} t={}", n, q, t);

    for profile in &["sequential", "zipf", "uniform"] {
        eprintln!("[validate] profile={}", profile);
        match t {
            2 => run_profile::<2>(profile, n, q),
            3 => run_profile::<3>(profile, n, q),
            4 => run_profile::<4>(profile, n, q),
            5 => run_profile::<5>(profile, n, q),
            6 => run_profile::<6>(profile, n, q),
            7 => run_profile::<7>(profile, n, q),
            8 => run_profile::<8>(profile, n, q),
            9 => run_profile::<9>(profile, n, q),
            10 => run_profile::<10>(profile, n, q),
            _ => panic!("unsupported T={}", t),
        }
    }
}
