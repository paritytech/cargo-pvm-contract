#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("U128Erc20.sol", allocator = "bump")]
mod u128_erc20 {
    use pvm_contract_sdk::{Address, Lazy, Mapping, SolDefaultError};

    #[derive(pvm_contract_sdk::SolEvent)]
    pub struct Transfer {
        #[indexed]
        pub from: Address,
        #[indexed]
        pub to: Address,
        pub value: u128,
    }

    #[derive(pvm_contract_sdk::SolEvent)]
    pub struct Approval {
        #[indexed]
        pub owner: Address,
        #[indexed]
        pub spender: Address,
        pub value: u128,
    }

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientAllowance;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub enum TokenError {
        InsufficientBalance(InsufficientBalance),
        InsufficientAllowance(InsufficientAllowance),
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
        pub fn total_supply(&self) -> u128 {
            self.total_supply.get()
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> u128 {
            self.balances.get(&account)
        }

        #[pvm_contract_sdk::method]
        pub fn allowance(&self, owner: Address, spender: Address) -> u128 {
            self.allowances.get(&owner).get(&spender)
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: u128) -> Result<(), TokenError> {
            let mut recipient = self.balances.entry(&to);
            let credited = recipient.get().wrapping_add(amount);
            recipient.set(&credited);

            let supply = self.total_supply.get().wrapping_add(amount);
            self.total_supply.set(&supply);

            self.emit_transfer(Address([0u8; 20]), to, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: u128) -> Result<(), TokenError> {
            let caller = self.env().caller();
            self.move_balance(caller, to, amount)?;
            self.emit_transfer(caller, to, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn approve(&mut self, spender: Address, amount: u128) -> Result<(), TokenError> {
            let caller = self.env().caller();
            self.allowances.entry(&caller).insert(&spender, &amount);
            self.emit_approval(caller, spender, amount);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn transfer_from(
            &mut self,
            from: Address,
            to: Address,
            amount: u128,
        ) -> Result<(), TokenError> {
            let caller = self.env().caller();

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

            let mut recipient = self.balances.entry(&to);
            let credited = recipient.get().wrapping_add(amount);
            recipient.set(&credited);
            Ok(())
        }

        fn emit_transfer(&self, from: Address, to: Address, value: u128) {
            Transfer { from, to, value }.emit(self.host());
        }

        fn emit_approval(&self, owner: Address, spender: Address, value: u128) {
            Approval {
                owner,
                spender,
                value,
            }
            .emit(self.host());
        }
    }
}
