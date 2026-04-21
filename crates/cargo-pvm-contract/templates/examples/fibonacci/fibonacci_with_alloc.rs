#![no_main]
#![no_std]

#[pvm_contract_macros::contract("Fibonacci.sol", allocator = "bump")]
mod fibonacci {
    use pvm_contract_types::{HostApi, PolkaVmHost};

    pub struct Fibonacci<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> Fibonacci<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_types::EmptyError> {
            Ok(())
        }

        #[pvm_contract_macros::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_types::EmptyError> {
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn fibonacci(&self, n: u32) -> u32 {
            if n <= 1 {
                n
            } else {
                let mut a = 0u32;
                let mut b = 1u32;
                for _ in 2..=n {
                    let c = a.wrapping_add(b);
                    a = b;
                    b = c;
                }
                b
            }
        }
    }
}
