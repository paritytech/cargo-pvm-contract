#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("U128Amm.sol", allocator = "bump")]
mod u128_amm {
    use pvm_contract_sdk::{EmptyError, Lazy, SolDefaultError, SolStorage, SolType};

    const FEE_NUMERATOR: u128 = 997;
    const FEE_DENOMINATOR: u128 = 1000;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientLiquidity;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub enum AmmError {
        InsufficientLiquidity(InsufficientLiquidity),
        SolDefaultError(SolDefaultError),
    }

    #[derive(SolType, SolStorage)]
    pub struct Reserves {
        pub reserve0: u128,
        pub reserve1: u128,
    }

    pub struct U128Amm {
        reserves: Lazy<Reserves>,
    }

    impl U128Amm {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_reserves(&self) -> (u128, u128) {
            let reserves = self.reserves.get();
            (reserves.reserve0, reserves.reserve1)
        }

        #[pvm_contract_sdk::method]
        pub fn sync(&mut self, reserve0: u128, reserve1: u128) -> Result<(), EmptyError> {
            self.reserves.set(&Reserves { reserve0, reserve1 });
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_amount_out(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
            amount_out(amount_in, reserve_in, reserve_out)
        }

        #[pvm_contract_sdk::method]
        pub fn get_amount_in(amount_out: u128, reserve_in: u128, reserve_out: u128) -> u128 {
            if reserve_out <= amount_out {
                return 0;
            }
            let numerator = reserve_in
                .wrapping_mul(amount_out)
                .wrapping_mul(FEE_DENOMINATOR);
            let denominator = (reserve_out - amount_out).wrapping_mul(FEE_NUMERATOR);
            (numerator / denominator).wrapping_add(1)
        }

        #[pvm_contract_sdk::method]
        pub fn swap_exact_in(
            &mut self,
            amount_in: u128,
            zero_for_one: bool,
        ) -> Result<u128, AmmError> {
            let reserves = self.reserves.get();
            let (reserve_in, reserve_out) = if zero_for_one {
                (reserves.reserve0, reserves.reserve1)
            } else {
                (reserves.reserve1, reserves.reserve0)
            };
            if reserve_in == 0 || reserve_out == 0 {
                return Err(InsufficientLiquidity.into());
            }

            let out = amount_out(amount_in, reserve_in, reserve_out);
            if out >= reserve_out {
                return Err(InsufficientLiquidity.into());
            }

            let new_in = reserve_in.wrapping_add(amount_in);
            let new_out = reserve_out - out;
            let updated = if zero_for_one {
                Reserves {
                    reserve0: new_in,
                    reserve1: new_out,
                }
            } else {
                Reserves {
                    reserve0: new_out,
                    reserve1: new_in,
                }
            };
            self.reserves.set(&updated);

            Ok(out)
        }

        #[pvm_contract_sdk::method]
        pub fn quote_cumulative(&self, amount_in: u128, hops: u32) -> u128 {
            let reserves = self.reserves.get();
            let mut amount = amount_in;
            for _ in 0..hops {
                amount = amount_out(amount, reserves.reserve0, reserves.reserve1);
            }
            amount
        }
    }

    fn amount_out(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
        let amount_in_with_fee = amount_in.wrapping_mul(FEE_NUMERATOR);
        let numerator = amount_in_with_fee.wrapping_mul(reserve_out);
        let denominator = reserve_in
            .wrapping_mul(FEE_DENOMINATOR)
            .wrapping_add(amount_in_with_fee);
        if denominator == 0 {
            0
        } else {
            numerator / denominator
        }
    }
}
