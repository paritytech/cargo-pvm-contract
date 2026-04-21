#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("Fibonacci.sol", allocator = "bump")]
mod fibonacci {
    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    #[pvm_contract_sdk::method]
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
