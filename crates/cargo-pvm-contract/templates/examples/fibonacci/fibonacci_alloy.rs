#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec;
use alloy_core::sol_types::{SolType, sol_data};
use pallet_revive_uapi::{HostFn, HostFnImpl as api, ReturnFlags};

#[global_allocator]
static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {
    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);
    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
        picoalloc::ArrayPointer::new(&raw mut ARRAY)
    }))
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

const FIBONACCI_SELECTOR: [u8; 4] = [0xe4, 0x44, 0xa7, 0x09];

fn fibonacci(n: u32) -> u32 {
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
        FIBONACCI_SELECTOR => {
            let n = <sol_data::Uint<32> as SolType>::abi_decode(input, true).unwrap();
            let result = fibonacci(n);
            let encoded = <sol_data::Uint<32> as SolType>::abi_encode(&result);
            api::return_value(ReturnFlags::empty(), &encoded);
        }
        _ => {}
    }
}
