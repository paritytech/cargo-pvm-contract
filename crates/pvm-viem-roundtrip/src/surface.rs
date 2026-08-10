//! A contract that exists purely to exercise the ABI emitter.
//!
//! The 20 contracts under `examples/test-contracts` cover the `.sol` path well,
//! but they leave large parts of the *Rust* path — the one that builds ABI JSON
//! from `SolEncode::abi_param` rather than from a Solidity interface — with no
//! coverage at all: signed integers, `bytesN` widths, `bytes` next to
//! `uint8[]`, fixed arrays of dynamic elements, dynamic structs, parameterised
//! errors, indexed dynamic event fields, anonymous events, overloads.
//!
//! Rather than add another binary to the riscv build (and to every E2E run),
//! the contract lives here as `#[contract(no_main)]`: the macro still emits the
//! `__abi_json()` accessor, so the fixture generator gets a real emitter output
//! at host-build cost. The method bodies are never executed — only their
//! signatures reach the ABI.

#[pvm_contract_sdk::contract(no_main, allocator = "pico")]
pub mod abi_surface {
    use pvm_contract_sdk::{
        Address, Bytes, I256, Panic, RevertString, SolError, SolEvent, SolType, U256,
    };

    /// Fully static struct: one `tuple` parameter whose components are all
    /// fixed-width, so it encodes inline with no offset.
    #[derive(SolType, Debug, PartialEq)]
    pub struct Pair {
        pub lo: u64,
        pub hi: u64,
    }

    /// Struct with dynamic members: still one `tuple` parameter, but the body
    /// carries a head/tail split that viem has to reproduce exactly.
    #[derive(SolType, Debug, PartialEq)]
    pub struct Profile {
        pub id: U256,
        pub name: String,
        pub tags: Vec<u32>,
    }

    /// Parameterless custom error.
    #[derive(Debug, SolError)]
    pub struct Unauthorized;

    /// Custom error with static fields — the shape OpenZeppelin-style errors
    /// use, and the one `decodeErrorResult` has to return named args for.
    #[derive(Debug, SolError)]
    pub struct InsufficientBalance {
        pub account: Address,
        pub required: U256,
        pub available: U256,
    }

    /// Custom error with a dynamic field, so the revert payload has its own
    /// head/tail split rather than being a flat run of words.
    #[derive(Debug, SolError)]
    pub struct DetailedFailure {
        pub reason: String,
        pub code: u32,
    }

    /// Error enum: its own selector is zeroed and the wire selector is always
    /// the held variant's, so this is what proves viem resolves the *variant*
    /// rather than the enum.
    #[derive(Debug, SolError)]
    pub enum SurfaceError {
        Unauthorized(Unauthorized),
        InsufficientBalance(InsufficientBalance),
        DetailedFailure(DetailedFailure),
        Panic(Panic),
        Revert(RevertString),
    }

    /// Three indexed fields — the non-anonymous maximum — spanning the
    /// right-aligned (`uint256`), left-aligned (`bytes32`) and zero-padded
    /// (`address`) topic encodings.
    #[derive(SolEvent)]
    pub struct Indexed3 {
        #[indexed]
        pub who: Address,
        #[indexed]
        pub amount: U256,
        #[indexed]
        pub tag: [u8; 32],
        pub note: u64,
    }

    /// Indexed dynamic fields hash to `keccak256(raw_bytes)`, so the value is
    /// not recoverable from the log. `#[alloc]` is required because the
    /// non-indexed field is dynamic.
    #[derive(SolEvent)]
    #[alloc]
    pub struct IndexedDynamic {
        #[indexed]
        pub name: String,
        #[indexed]
        pub payload: Bytes,
        pub note: String,
    }

    /// An indexed static composite hashes to `keccak256(abi.encode(value))`.
    #[derive(SolEvent)]
    #[alloc]
    pub struct IndexedComposite {
        #[indexed]
        pub pair: Pair,
        pub values: Vec<U256>,
    }

    /// Anonymous events carry no signature topic and may index four fields.
    #[derive(SolEvent)]
    #[anonymous]
    pub struct AnonymousPing {
        #[indexed]
        pub a: U256,
        #[indexed]
        pub b: Address,
        pub c: bool,
    }

    pub struct AbiSurface;

    impl AbiSurface {
        /// Constructor with arguments: emitted as a `constructor` ABI entry
        /// with inputs, which is what `encodeDeployData` consumes.
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self, owner: Address, supply: U256) -> Result<(), SurfaceError> {
            let _ = (owner, supply);
            Ok(())
        }

        /// No receiver: `pure`. Present so all four `stateMutability` values
        /// appear in a single ABI.
        #[pvm_contract_sdk::method]
        pub fn version() -> u32 {
            1
        }

        /// Every signed width in one signature. Negative values are
        /// sign-extended, which is the encoding viem has to match.
        #[pvm_contract_sdk::method]
        pub fn echo_ints(&self, a: i8, b: i16, c: i32, d: i64, e: i128, f: I256) -> I256 {
            let _ = (a, b, c, d, e);
            f
        }

        /// `bytesN` is left-aligned, unlike every integer type.
        #[pvm_contract_sdk::method]
        pub fn echo_bytes_n(
            &self,
            a: [u8; 1],
            b: [u8; 4],
            c: [u8; 20],
            d: [u8; 32],
        ) -> ([u8; 4], [u8; 32]) {
            let _ = (a, c);
            (b, d)
        }

        /// `Bytes` and `Vec<u8>` are the same Rust shape but different Solidity
        /// types (`bytes` vs `uint8[]`) with very different layouts.
        #[pvm_contract_sdk::method]
        pub fn echo_bytes_vs_uint8(&self, packed: Bytes, spread: Vec<u8>) -> (Bytes, Vec<u8>) {
            (packed, spread)
        }

        /// Array of a dynamic element: an offset table inside the array body.
        #[pvm_contract_sdk::method]
        pub fn echo_strings(&self, xs: Vec<String>) -> Vec<String> {
            xs
        }

        /// Array of tuples.
        #[pvm_contract_sdk::method]
        pub fn echo_pairs(&self, xs: Vec<Pair>) -> Vec<Pair> {
            xs
        }

        /// Fixed-length array of a dynamic element: static length, dynamic
        /// body, so the parameter as a whole is dynamic.
        #[pvm_contract_sdk::method]
        pub fn echo_fixed_strings(&self, xs: [String; 2]) -> [String; 2] {
            xs
        }

        /// Fixed-length array of a static element: encodes flat, no offset.
        #[pvm_contract_sdk::method]
        pub fn echo_fixed_uints(&self, xs: [U256; 3]) -> [U256; 3] {
            xs
        }

        /// Multi-return where one member is dynamic: two ABI outputs with a
        /// head/tail split, decoded by viem as an array.
        #[pvm_contract_sdk::method]
        pub fn mixed(&self, id: U256, name: String) -> (U256, String) {
            (id, name)
        }

        /// Single `tuple` output with named components, decoded by viem as an
        /// object rather than an array.
        #[pvm_contract_sdk::method]
        pub fn echo_pair(&self, p: Pair) -> Pair {
            p
        }

        /// Dynamic struct, in and out.
        #[pvm_contract_sdk::method]
        pub fn echo_profile(&self, p: Profile) -> Profile {
            p
        }

        /// `nonpayable`: mutating, no value accepted.
        #[pvm_contract_sdk::method]
        pub fn touch(&mut self) {}

        /// `payable`.
        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::payable]
        pub fn deposit(&mut self) {}

        /// Reverts with the error enum, so the fixture's revert payloads have a
        /// method to belong to.
        #[pvm_contract_sdk::method]
        pub fn always_fails(&self) -> Result<(), SurfaceError> {
            Err(Unauthorized.into())
        }

        /// Registers the OpenZeppelin-compatible
        /// `ReentrancyGuardReentrantCall()` error in the ABI, which is the one
        /// SDK revert Foundry and Etherscan are expected to decode by name.
        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn guarded(&mut self) -> Result<(), SurfaceError> {
            Ok(())
        }

        /// Overload pair: same ABI name, different parameter types, therefore
        /// different selectors. viem picks the item by argument shape.
        #[pvm_contract_sdk::method(rename = "overloaded")]
        pub fn overloaded_uint(&self, v: U256) -> U256 {
            v
        }

        #[pvm_contract_sdk::method(rename = "overloaded")]
        pub fn overloaded_string(&self, v: String) -> U256 {
            U256::from(v.len() as u64)
        }

        /// Emits a `receive` ABI entry.
        #[pvm_contract_sdk::receive]
        pub fn receive(&mut self) {}

        /// Emits nothing in the ABI today — the emitter has no `fallback`
        /// variant — which is itself asserted by the TypeScript suite.
        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), SurfaceError> {
            Ok(())
        }
    }
}
