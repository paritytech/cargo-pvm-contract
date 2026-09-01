#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("U128Erc20.sol", allocator = "bump")]
mod u128_erc20 {
    use pvm_contract_sdk::{Address, Lazy, Mapping, SolDefaultError, U256};

    #[derive(pvm_contract_sdk::SolEvent)]
    pub struct Transfer {
        #[indexed]
        pub from: Address,
        #[indexed]
        pub to: Address,
        pub value: U256,
    }

    #[derive(pvm_contract_sdk::SolEvent)]
    pub struct Approval {
        #[indexed]
        pub owner: Address,
        #[indexed]
        pub spender: Address,
        pub value: U256,
    }

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientAllowance;

    /// A `uint256` amount, or the balance or supply it would produce, does not
    /// fit the 128-bit interior this token keeps in storage.
    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct AmountTooLarge;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub enum TokenError {
        InsufficientBalance(InsufficientBalance),
        InsufficientAllowance(InsufficientAllowance),
        AmountTooLarge(AmountTooLarge),
        SolDefaultError(SolDefaultError),
    }

    // Auto-numbered: explicit `#[slot(N)]` is rejected for sub-word types like
    // `Lazy<u128>`, which solc places right-aligned inside its slot.
    pub struct U128Erc20 {
        total_supply: Lazy<u128>,
        balances: Mapping<Address, u128>,
        allowances: Mapping<Address, Mapping<Address, u128>>,
    }

    impl U128Erc20 {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), TokenError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn total_supply(&self) -> U256 {
            U256::from(self.total_supply.get())
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            U256::from(self.balances.get(&account))
        }

        #[pvm_contract_sdk::method]
        pub fn allowance(&self, owner: Address, spender: Address) -> U256 {
            U256::from(self.allowances.get(&owner).get(&spender))
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let amount = narrow_to_u128(amount)?;

            let supply = self
                .total_supply
                .get()
                .checked_add(amount)
                .ok_or(AmountTooLarge)?;
            self.total_supply.set(&supply);
            self.credit(to, amount)?;

            self.emit_transfer(Address([0u8; 20]), to, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let caller = self.env().caller();
            let amount = narrow_to_u128(amount)?;
            self.move_balance(caller, to, amount)?;
            self.emit_transfer(caller, to, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn approve(&mut self, spender: Address, amount: U256) -> Result<(), TokenError> {
            let caller = self.env().caller();
            let amount = narrow_to_u128(amount)?;
            self.allowances.entry(&caller).insert(&spender, &amount);
            self.emit_approval(caller, spender, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn transfer_from(
            &mut self,
            from: Address,
            to: Address,
            amount: U256,
        ) -> Result<(), TokenError> {
            let caller = self.env().caller();
            let amount = narrow_to_u128(amount)?;

            let mut owner_allowances = self.allowances.entry(&from);
            let mut allowance = owner_allowances.entry(&caller);
            let remaining = allowance.get();
            if remaining < amount {
                return Err(InsufficientAllowance.into());
            }
            allowance.set(&(remaining - amount));
            drop(allowance);
            drop(owner_allowances);

            self.move_balance(from, to, amount)?;
            self.emit_transfer(from, to, amount);
            Ok(())
        }

        fn move_balance(
            &mut self,
            from: Address,
            to: Address,
            amount: u128,
        ) -> Result<(), TokenError> {
            let mut sender = self.balances.entry(&from);
            let sender_balance = sender.get();
            if sender_balance < amount {
                return Err(InsufficientBalance.into());
            }
            sender.set(&(sender_balance - amount));
            drop(sender);

            self.credit(to, amount)
        }

        fn credit(&mut self, to: Address, amount: u128) -> Result<(), TokenError> {
            let mut recipient = self.balances.entry(&to);
            let credited = recipient.get().checked_add(amount).ok_or(AmountTooLarge)?;
            recipient.set(&credited);
            Ok(())
        }

        fn emit_transfer(&self, from: Address, to: Address, value: u128) {
            Transfer {
                from,
                to,
                value: U256::from(value),
            }
            .emit(self.host());
        }

        fn emit_approval(&self, owner: Address, spender: Address, value: u128) {
            Approval {
                owner,
                spender,
                value: U256::from(value),
            }
            .emit(self.host());
        }
    }

    /// Narrows an amount arriving over the `uint256` ABI to the 128-bit
    /// interior, reverting when it does not fit.
    fn narrow_to_u128(amount: U256) -> Result<u128, TokenError> {
        u128::try_from(amount).map_err(|_| AmountTooLarge.into())
    }
}
