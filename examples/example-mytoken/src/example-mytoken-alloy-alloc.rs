#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec;
use alloy_core::primitives::U256;
use alloy_core::sol_types::{sol_data, SolType};
use pvm_contract_sdk::pallet_revive_uapi::{HostFn as _, HostFnImpl as api};
use pvm_contract_sdk::{ReturnFlags, StorageFlags};

#[global_allocator]
static ALLOC: pvm_bump_allocator::BumpAllocator<1024> = pvm_bump_allocator::BumpAllocator::new();

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

const TOTAL_SUPPLY_SELECTOR: [u8; 4] = [0x18, 0x16, 0x0d, 0xdd];
const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
const MINT_SELECTOR: [u8; 4] = [0x40, 0xc1, 0x0f, 0x19];

const TRANSFER_EVENT_SIGNATURE: [u8; 32] = [
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
];

fn total_supply_key() -> [u8; 32] {
    [0u8; 32]
}

fn balance_key(addr: &[u8; 20]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[12..32].copy_from_slice(addr);
    input[63] = 1;

    let mut key = [0u8; 32];
    api::hash_keccak_256(&input, &mut key);
    key
}

fn set_total_supply(amount: U256) {
    let key = total_supply_key();
    api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
}

fn set_balance(addr: &[u8; 20], amount: U256) {
    let key = balance_key(addr);
    api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
}

fn get_caller() -> [u8; 20] {
    let mut caller = [0u8; 20];
    api::caller(&mut caller);
    caller
}

fn emit_transfer(from: &[u8; 20], to: &[u8; 20], value: U256) {
    let mut from_topic = [0u8; 32];
    from_topic[12..32].copy_from_slice(from);

    let mut to_topic = [0u8; 32];
    to_topic[12..32].copy_from_slice(to);

    let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
    let data = value.to_be_bytes::<32>();
    api::deposit_event(&topics, &data);
}

#[polkavm_derive::polkavm_export]
extern "C" fn deploy() {}

#[polkavm_derive::polkavm_export]
extern "C" fn call() {
    let call_data_len = api::call_data_size() as usize;
    let mut call_data = vec![0u8; call_data_len];
    api::call_data_copy(&mut call_data, 0);

    if call_data_len < 4 {
        return;
    }

    let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
    let input = &call_data[4..];

    match selector {
        TOTAL_SUPPLY_SELECTOR => {
            let key = total_supply_key();
            let mut supply_bytes = vec![0u8; 32];
            let mut supply_output = supply_bytes.as_mut_slice();

            let result = match api::get_storage(StorageFlags::empty(), &key, &mut supply_output) {
                Ok(_) => U256::from_be_bytes::<32>(supply_output[0..32].try_into().unwrap()),
                Err(_) => U256::ZERO,
            };
            let encoded = <sol_data::Uint<256> as SolType>::abi_encode(&result);
            api::return_value(ReturnFlags::empty(), &encoded);
        }
        BALANCE_OF_SELECTOR => {
            let addr = <sol_data::Address as SolType>::abi_decode(input).unwrap();
            let key = balance_key(addr.as_ref());
            let mut balance_bytes = vec![0u8; 32];
            let mut balance_output = balance_bytes.as_mut_slice();

            let result = match api::get_storage(StorageFlags::empty(), &key, &mut balance_output) {
                Ok(_) => U256::from_be_bytes::<32>(balance_output[0..32].try_into().unwrap()),
                Err(_) => U256::ZERO,
            };
            let encoded = <sol_data::Uint<256> as SolType>::abi_encode(&result);
            api::return_value(ReturnFlags::empty(), &encoded);
        }
        TRANSFER_SELECTOR => {
            type TransferArgs = (sol_data::Address, sol_data::Uint<256>);
            let (to, amount) = <TransferArgs as SolType>::abi_decode(input).unwrap();
            let to: [u8; 20] = *to.as_ref();

            let caller = get_caller();
            let sender_key = balance_key(&caller);
            let mut sender_balance_bytes = vec![0u8; 32];
            let mut sender_balance_output = sender_balance_bytes.as_mut_slice();
            let sender_balance = match api::get_storage(
                StorageFlags::empty(),
                &sender_key,
                &mut sender_balance_output,
            ) {
                Ok(_) => {
                    U256::from_be_bytes::<32>(sender_balance_output[0..32].try_into().unwrap())
                }
                Err(_) => U256::ZERO,
            };

            if sender_balance < amount {
                api::return_value(ReturnFlags::REVERT, b"InsufficientBalance");
            }

            let new_sender_balance = sender_balance - amount;
            let recipient_key = balance_key(&to);
            let mut recipient_balance_bytes = vec![0u8; 32];
            let mut recipient_balance_output = recipient_balance_bytes.as_mut_slice();
            let recipient_balance = match api::get_storage(
                StorageFlags::empty(),
                &recipient_key,
                &mut recipient_balance_output,
            ) {
                Ok(_) => {
                    U256::from_be_bytes::<32>(recipient_balance_output[0..32].try_into().unwrap())
                }
                Err(_) => U256::ZERO,
            };
            let new_recipient_balance = recipient_balance + amount;

            set_balance(&caller, new_sender_balance);
            set_balance(&to, new_recipient_balance);
            emit_transfer(&caller, &to, amount);
        }
        MINT_SELECTOR => {
            type MintArgs = (sol_data::Address, sol_data::Uint<256>);
            let (to, amount) = <MintArgs as SolType>::abi_decode(input).unwrap();
            let to: [u8; 20] = *to.as_ref();

            let recipient_key = balance_key(&to);
            let mut recipient_balance_bytes = vec![0u8; 32];
            let mut recipient_balance_output = recipient_balance_bytes.as_mut_slice();
            let recipient_balance = match api::get_storage(
                StorageFlags::empty(),
                &recipient_key,
                &mut recipient_balance_output,
            ) {
                Ok(_) => {
                    U256::from_be_bytes::<32>(recipient_balance_output[0..32].try_into().unwrap())
                }
                Err(_) => U256::ZERO,
            };
            let new_recipient_balance = recipient_balance.saturating_add(amount);
            set_balance(&to, new_recipient_balance);

            let supply_key = total_supply_key();
            let mut supply_bytes = vec![0u8; 32];
            let mut supply_output = supply_bytes.as_mut_slice();
            let supply =
                match api::get_storage(StorageFlags::empty(), &supply_key, &mut supply_output) {
                    Ok(_) => U256::from_be_bytes::<32>(supply_output[0..32].try_into().unwrap()),
                    Err(_) => U256::ZERO,
                };
            let new_supply = supply.saturating_add(amount);
            set_total_supply(new_supply);

            emit_transfer(&[0u8; 20], &to, amount);
        }
        _ => {}
    }
}
