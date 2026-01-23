#![no_main]
#![no_std]

use pvm_contract::api;

#[pvm_contract::contract("Fibonacci.sol", no_alloc, buffer = 256)]
mod fibonacci {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {}

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {}
        }
    }

    #[pvm_contract::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract::method]
    pub fn fibonacci(n: u32) -> u32 {
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
