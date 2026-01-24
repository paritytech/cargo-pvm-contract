#![no_main]
#![no_std]

#[global_allocator]
static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {
    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);
    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
        picoalloc::ArrayPointer::new(&raw mut ARRAY)
    }))
};

#[pvm_contract_macros::contract("Fibonacci.sol")]
mod fibonacci {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {}

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {}
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
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
