#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("U128Amm.sol", allocator = "bump")]
mod u128_amm {
    use pvm_contract_sdk::{EmptyError, Lazy, SolDefaultError, SolStorage, SolType, U256};

    const FEE_NUMERATOR: u128 = 997;
    const FEE_DENOMINATOR: u128 = 1000;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientLiquidity;

    /// A `uint256` argument, or a quote computed from it, does not fit the
    /// 128-bit reserves this pool keeps in storage.
    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct AmountTooLarge;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub enum AmmError {
        InsufficientLiquidity(InsufficientLiquidity),
        AmountTooLarge(AmountTooLarge),
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
        pub fn get_reserves(&self) -> (U256, U256) {
            let reserves = self.reserves.get();
            (U256::from(reserves.reserve0), U256::from(reserves.reserve1))
        }

        #[pvm_contract_sdk::method]
        pub fn sync(&mut self, reserve0: U256, reserve1: U256) -> Result<(), AmmError> {
            self.reserves.set(&Reserves {
                reserve0: narrow_to_u128(reserve0)?,
                reserve1: narrow_to_u128(reserve1)?,
            });
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_amount_out(
            amount_in: U256,
            reserve_in: U256,
            reserve_out: U256,
        ) -> Result<U256, AmmError> {
            let quote = quote_out(
                narrow_to_u128(amount_in)?,
                narrow_to_u128(reserve_in)?,
                narrow_to_u128(reserve_out)?,
            )?;
            Ok(U256::from(quote))
        }

        #[pvm_contract_sdk::method]
        pub fn get_amount_in(
            amount_out: U256,
            reserve_in: U256,
            reserve_out: U256,
        ) -> Result<U256, AmmError> {
            let quote = quote_in(
                narrow_to_u128(amount_out)?,
                narrow_to_u128(reserve_in)?,
                narrow_to_u128(reserve_out)?,
            )?;
            Ok(U256::from(quote))
        }

        #[pvm_contract_sdk::method]
        pub fn swap_exact_in(
            &mut self,
            amount_in: U256,
            zero_for_one: bool,
        ) -> Result<U256, AmmError> {
            let amount_in = narrow_to_u128(amount_in)?;
            let reserves = self.reserves.get();
            let (reserve_in, reserve_out) = if zero_for_one {
                (reserves.reserve0, reserves.reserve1)
            } else {
                (reserves.reserve1, reserves.reserve0)
            };

            let out = quote_out(amount_in, reserve_in, reserve_out)?;
            if out >= reserve_out {
                return Err(InsufficientLiquidity.into());
            }

            let new_in = reserve_in.checked_add(amount_in).ok_or(AmountTooLarge)?;
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

            Ok(U256::from(out))
        }

        #[pvm_contract_sdk::method]
        pub fn quote_cumulative(&self, amount_in: U256, hops: u32) -> Result<U256, AmmError> {
            let reserves = self.reserves.get();
            let mut amount = narrow_to_u128(amount_in)?;
            for _ in 0..hops {
                amount = quote_out(amount, reserves.reserve0, reserves.reserve1)?;
            }
            Ok(U256::from(amount))
        }
    }

    /// Output for an exact input, rounded down. `amount_in * 997 * reserve_out`
    /// needs 266 bits at the top of the `u128` range, so every product is taken
    /// at 256 bits and checked.
    fn quote_out(amount_in: u128, reserve_in: u128, reserve_out: u128) -> Result<u128, AmmError> {
        if reserve_in == 0 || reserve_out == 0 {
            return Err(InsufficientLiquidity.into());
        }

        let amount_in_with_fee = U256::from(amount_in)
            .checked_mul(U256::from(FEE_NUMERATOR))
            .ok_or(AmountTooLarge)?;
        let numerator = amount_in_with_fee
            .checked_mul(U256::from(reserve_out))
            .ok_or(AmountTooLarge)?;
        let denominator = U256::from(reserve_in)
            .checked_mul(U256::from(FEE_DENOMINATOR))
            .ok_or(AmountTooLarge)?
            .checked_add(amount_in_with_fee)
            .ok_or(AmountTooLarge)?;

        narrow_to_u128(numerator / denominator)
    }

    /// Input required for an exact output, rounded up.
    fn quote_in(amount_out: u128, reserve_in: u128, reserve_out: u128) -> Result<u128, AmmError> {
        if reserve_in == 0 || reserve_out <= amount_out {
            return Err(InsufficientLiquidity.into());
        }

        let numerator = U256::from(reserve_in)
            .checked_mul(U256::from(amount_out))
            .ok_or(AmountTooLarge)?
            .checked_mul(U256::from(FEE_DENOMINATOR))
            .ok_or(AmountTooLarge)?;
        let denominator = U256::from(reserve_out - amount_out)
            .checked_mul(U256::from(FEE_NUMERATOR))
            .ok_or(AmountTooLarge)?;

        narrow_to_u128(numerator / denominator)?
            .checked_add(1)
            .ok_or(AmountTooLarge.into())
    }

    /// Narrows a value arriving over the `uint256` ABI, or a 256-bit quote, to
    /// the pool's 128-bit interior, reverting when it does not fit.
    fn narrow_to_u128(value: U256) -> Result<u128, AmmError> {
        u128::try_from(value).map_err(|_| AmountTooLarge.into())
    }
}
