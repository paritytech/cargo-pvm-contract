#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract(allocator = "pico", allocator_size = 16384)]
mod ordered_index_bench {
    use alloc::string::String;
    use core::ops::Bound;

    use pvm_storage::ordered_index::OrderedIndex;

    pub struct OrderedIndexBench;

    impl OrderedIndexBench {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::SolDefaultError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn insert(&mut self, key: String, value: u64) {
            let host = self.host.clone();
            let idx = OrderedIndex::<String, u64, 10>::new(b"oibench", host.clone());
            let _ = idx.insert(&host, &key, &value);
        }

        #[pvm_contract_sdk::method]
        pub fn range_query(&self, prefix: String) -> u64 {
            let host = self.host.clone();
            let idx = OrderedIndex::<String, u64, 10>::new(b"oibench", host.clone());
            let upper = {
                let mut s = prefix.clone();
                s.push('\u{FF}');
                s
            };
            let results = idx.range(
                &host,
                Bound::Included(&prefix),
                Bound::Excluded(&upper),
                0,
                100,
            );
            results.len() as u64
        }
    }
}