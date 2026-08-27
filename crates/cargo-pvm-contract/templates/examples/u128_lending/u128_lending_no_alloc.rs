#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("U128Lending.sol", buffer = 256)]
mod u128_lending {
    use pvm_contract_sdk::{EmptyError, Lazy};

    const WAD: u128 = 1_000_000_000_000_000_000;
    const Q64_ONE: u128 = 1 << 64;

    pub struct U128Lending {
        principal: Lazy<u128>,
    }

    impl U128Lending {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn principal(&self) -> u128 {
            self.principal.get()
        }

        #[pvm_contract_sdk::method]
        pub fn set_principal(&mut self, amount: u128) -> Result<(), EmptyError> {
            self.principal.set(&amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn mul_div_down(x: u128, y: u128, denominator: u128) -> u128 {
            mul_div_down(x, y, denominator)
        }

        #[pvm_contract_sdk::method]
        pub fn mul_div_up(x: u128, y: u128, denominator: u128) -> u128 {
            mul_div_up(x, y, denominator)
        }

        #[pvm_contract_sdk::method]
        pub fn accrue(
            &mut self,
            rate_per_period_wad: u128,
            periods: u32,
        ) -> Result<u128, EmptyError> {
            let growth = WAD.wrapping_add(rate_per_period_wad);
            let mut balance = self.principal.get();
            for _ in 0..periods {
                balance = mul_div_up(balance, growth, WAD);
            }
            self.principal.set(&balance);
            Ok(balance)
        }

        #[pvm_contract_sdk::method]
        pub fn compound_q64(&self, rate_q64: u128, periods: u32) -> u128 {
            let growth = Q64_ONE.wrapping_add(rate_q64);
            let mut balance = self.principal.get();
            for _ in 0..periods {
                balance = balance.wrapping_mul(growth) >> 64;
            }
            balance
        }

        #[pvm_contract_sdk::method]
        pub fn utilization_wad(borrows: u128, supply: u128) -> u128 {
            mul_div_down(borrows, WAD, supply)
        }
    }

    fn mul_div_down(x: u128, y: u128, denominator: u128) -> u128 {
        if denominator == 0 {
            return 0;
        }
        x.wrapping_mul(y) / denominator
    }

    fn mul_div_up(x: u128, y: u128, denominator: u128) -> u128 {
        if denominator == 0 {
            return 0;
        }
        let product = x.wrapping_mul(y);
        let quotient = product / denominator;
        if product % denominator == 0 {
            quotient
        } else {
            quotient.wrapping_add(1)
        }
    }
}
