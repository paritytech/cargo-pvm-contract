#![no_main]
#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {}

impl AsRef<[u8]> for Error {
    fn as_ref(&self) -> &[u8] {
        match *self {}
    }
}

pvm_contract_builder_dsl::pvm_contract! {
    no_alloc(buffer = 256);

    constructor fn new() -> Result<(), Error> {
        Ok(())
    }

    fallback fn fallback() -> Result<(), Error> {
        Ok(())
    }

    #[method("fibonacci(uint32)", returns(u32))]
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
}
