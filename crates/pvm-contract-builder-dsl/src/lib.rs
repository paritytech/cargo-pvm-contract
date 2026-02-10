#![no_std]

pub use pallet_revive_uapi;
pub use polkavm_derive;
pub use pvm_contract_types;
pub use ruint;
pub use tiny_keccak;

mod selector;

pub use selector::compute_selector;

/// Compute a Solidity function selector from a canonical signature.
///
/// Usage: `selector!("fibonacci(uint32)")` → `[u8; 4]`
#[macro_export]
macro_rules! selector {
    ($sig:expr) => {{ $crate::compute_selector($sig) }};
}

/// Define a PVM smart contract using a declarative macro DSL.
///
/// # Example (no-alloc mode)
///
/// ```ignore
/// pvm_contract! {
///     no_alloc(buffer = 256);
///
///     constructor fn new() -> Result<(), Error> { Ok(()) }
///     fallback fn fallback() -> Result<(), Error> { Ok(()) }
///
///     #[method("fibonacci(uint32)")]
///     fn fibonacci(n: u32) -> u32 { /* ... */ }
///
///     #[method("transfer(address,uint256)", result)]
///     fn transfer(to: [u8; 20], amount: U256) -> Result<(), Error> { /* ... */ }
/// }
/// ```
///
/// Methods returning `Result<(), E>` without an ABI-encoded value use `result`.
/// Methods with an ABI return value use `returns(Type)`.
#[macro_export]
macro_rules! pvm_contract {
    // no_alloc mode
    (
        no_alloc(buffer = $buf_size:expr);
        $(constructor fn $ctor_name:ident() -> Result<(), $ctor_err:ty> $ctor_body:block)?
        $(fallback fn $fb_name:ident() -> Result<(), $fb_err:ty> $fb_body:block)?
        $( #[method($sol_sig:expr $(, $($args:tt)*)?)]
           fn $method_name:ident($($param_name:ident : $param_type:ty),*) $(-> $fn_ret:ty)? $method_body:block
        )*
    ) => {
        $crate::__pvm_impl_body! {
            @mode no_alloc;
            @buffer $buf_size;
            @constructor [ $( fn $ctor_name() -> Result<(), $ctor_err> $ctor_body )? ];
            @fallback [ $( fn $fb_name() -> Result<(), $fb_err> $fb_body )? ];
            @methods [
                $( {
                    @sig $sol_sig;
                    @args [ $($($args)*)? ];
                    fn $method_name($($param_name : $param_type),*) $(-> $fn_ret)? $method_body
                } )*
            ];
        }
    };

    // alloc mode
    (
        alloc;
        $(constructor fn $ctor_name:ident() -> Result<(), $ctor_err:ty> $ctor_body:block)?
        $(fallback fn $fb_name:ident() -> Result<(), $fb_err:ty> $fb_body:block)?
        $( #[method($sol_sig:expr $(, $($args:tt)*)?)]
           fn $method_name:ident($($param_name:ident : $param_type:ty),*) $(-> $fn_ret:ty)? $method_body:block
        )*
    ) => {
        $crate::__pvm_impl_body! {
            @mode alloc;
            @buffer 0;
            @constructor [ $( fn $ctor_name() -> Result<(), $ctor_err> $ctor_body )? ];
            @fallback [ $( fn $fb_name() -> Result<(), $fb_err> $fb_body )? ];
            @methods [
                $( {
                    @sig $sol_sig;
                    @args [ $($($args)*)? ];
                    fn $method_name($($param_name : $param_type),*) $(-> $fn_ret)? $method_body
                } )*
            ];
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_impl_body {
    (
        @mode $mode:ident;
        @buffer $buf_size:expr;
        @constructor [ $( fn $ctor_name:ident() -> Result<(), $ctor_err:ty> $ctor_body:block )? ];
        @fallback [ $( fn $fb_name:ident() -> Result<(), $fb_err:ty> $fb_body:block )? ];
        @methods [
            $( {
                @sig $sol_sig:expr;
                @args [ $($arg:tt)* ];
                fn $method_name:ident($($param_name:ident : $param_type:ty),*) $(-> $fn_ret:ty)? $method_body:block
            } )*
        ];
    ) => {
        use $crate::pallet_revive_uapi::HostFn as _;

        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            unsafe {
                core::arch::asm!("unimp");
                core::hint::unreachable_unchecked()
            }
        }

        $( fn $ctor_name() -> Result<(), $ctor_err> $ctor_body )?
        $( fn $fb_name() -> Result<(), $fb_err> $fb_body )?
        $(
            #[inline(always)]
            fn $method_name($($param_name : $param_type),*) $(-> $fn_ret)? $method_body
        )*

        $crate::__pvm_deploy! { $( constructor $ctor_name; )? }

        $crate::__pvm_call! {
            @mode $mode;
            @buffer $buf_size;
            @fallback [ $( $fb_name; )? ];
            @methods [
                $( {
                    @sig $sol_sig;
                    @args [ $($arg)* ];
                    @fn_name $method_name;
                    @params [ $($param_name : $param_type),* ];
                } )*
            ];
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_deploy {
    (constructor $ctor_name:ident;) => {
        #[$crate::polkavm_derive::polkavm_export]
        pub extern "C" fn deploy() {
            match $ctor_name() {
                Ok(()) => {}
                Err(e) => {
                    $crate::pallet_revive_uapi::HostFnImpl::return_value(
                        $crate::pallet_revive_uapi::ReturnFlags::REVERT,
                        e.as_ref(),
                    );
                }
            }
        }
    };
    () => {
        #[$crate::polkavm_derive::polkavm_export]
        pub extern "C" fn deploy() {}
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_call {
    (
        @mode no_alloc;
        @buffer $buf_size:expr;
        @fallback [ $( $fb_name:ident; )? ];
        @methods [ $( $method:tt )* ];
    ) => {
        #[$crate::polkavm_derive::polkavm_export]
        pub extern "C" fn call() {
            let call_data_len = $crate::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;

            let mut call_data = [0u8; $buf_size];
            if call_data_len > $buf_size {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT,
                    b"CalldataTooLarge",
                );
            }
            $crate::pallet_revive_uapi::HostFnImpl::call_data_copy(
                &mut call_data[..call_data_len], 0,
            );

            if call_data_len < 4 {
                $crate::__pvm_fallback!($( $fb_name )?);
            }

            let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
            let input = &call_data[4..call_data_len];

            $crate::__pvm_dispatch!(selector, input; $($method)*);

            $crate::__pvm_fallback!($( $fb_name )?);
        }
    };

    (
        @mode alloc;
        @buffer $buf_size:expr;
        @fallback [ $( $fb_name:ident; )? ];
        @methods [ $( $method:tt )* ];
    ) => {
        extern crate alloc;

        #[$crate::polkavm_derive::polkavm_export]
        pub extern "C" fn call() {
            let call_data_len = $crate::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
            let mut call_data = alloc::vec![0u8; call_data_len];
            $crate::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);

            if call_data_len < 4 {
                $crate::__pvm_fallback!($( $fb_name )?);
            }

            let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
            let input = &call_data[4..];

            $crate::__pvm_dispatch!(selector, input; $($method)*);

            $crate::__pvm_fallback!($( $fb_name )?);
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_fallback {
    ($fb_name:ident) => {
        match $fb_name() {
            Ok(()) => return,
            Err(e) => {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT,
                    e.as_ref(),
                );
            }
        }
    };
    () => {
        $crate::pallet_revive_uapi::HostFnImpl::return_value(
            $crate::pallet_revive_uapi::ReturnFlags::REVERT,
            b"",
        );
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_dispatch {
    ($sel:ident, $input:ident;) => {};

    (
        $sel:ident, $input:ident;
        {
            @sig $sol_sig:expr;
            @args [];
            @fn_name $method_name:ident;
            @params [ $($param_name:ident : $param_type:ty),* ];
        }
        $($rest:tt)*
    ) => {
        if $sel == $crate::selector!($sol_sig) {
            $crate::__pvm_decode_and_call!($input, $method_name; @params [ $($param_name : $param_type),* ]; @result_flag []; @encode_ret [];);
        }

        $crate::__pvm_dispatch!($sel, $input; $($rest)*);
    };

    (
        $sel:ident, $input:ident;
        {
            @sig $sol_sig:expr;
            @args [result];
            @fn_name $method_name:ident;
            @params [ $($param_name:ident : $param_type:ty),* ];
        }
        $($rest:tt)*
    ) => {
        if $sel == $crate::selector!($sol_sig) {
            $crate::__pvm_decode_and_call!($input, $method_name; @params [ $($param_name : $param_type),* ]; @result_flag [result]; @encode_ret [];);
        }

        $crate::__pvm_dispatch!($sel, $input; $($rest)*);
    };

    (
        $sel:ident, $input:ident;
        {
            @sig $sol_sig:expr;
            @args [returns($ret_enc:ty)];
            @fn_name $method_name:ident;
            @params [ $($param_name:ident : $param_type:ty),* ];
        }
        $($rest:tt)*
    ) => {
        if $sel == $crate::selector!($sol_sig) {
            $crate::__pvm_decode_and_call!($input, $method_name; @params [ $($param_name : $param_type),* ]; @result_flag []; @encode_ret [$ret_enc];);
        }

        $crate::__pvm_dispatch!($sel, $input; $($rest)*);
    };

    (
        $sel:ident, $input:ident;
        {
            @sig $sol_sig:expr;
            @args [result, returns($ret_enc:ty)];
            @fn_name $method_name:ident;
            @params [ $($param_name:ident : $param_type:ty),* ];
        }
        $($rest:tt)*
    ) => {
        if $sel == $crate::selector!($sol_sig) {
            $crate::__pvm_decode_and_call!($input, $method_name; @params [ $($param_name : $param_type),* ]; @result_flag [result]; @encode_ret [$ret_enc];);
        }

        $crate::__pvm_dispatch!($sel, $input; $($rest)*);
    };

    (
        $sel:ident, $input:ident;
        {
            @sig $sol_sig:expr;
            @args [returns($ret_enc:ty), result];
            @fn_name $method_name:ident;
            @params [ $($param_name:ident : $param_type:ty),* ];
        }
        $($rest:tt)*
    ) => {
        if $sel == $crate::selector!($sol_sig) {
            $crate::__pvm_decode_and_call!($input, $method_name; @params [ $($param_name : $param_type),* ]; @result_flag [result]; @encode_ret [$ret_enc];);
        }

        $crate::__pvm_dispatch!($sel, $input; $($rest)*);
    };
}

// 8 arms covering all combinations of: {no_params, with_params} × {no_result, result} × {no_encode, encode}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_decode_and_call {
    // no params, no result, no encode
    ($input:ident, $fn:ident; @params []; @result_flag []; @encode_ret [];) => {
        $fn(); return;
    };

    // no params, result, no encode
    ($input:ident, $fn:ident; @params []; @result_flag [result]; @encode_ret [];) => {
        match $fn() {
            Ok(()) => return,
            Err(e) => {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
            }
        }
    };

    // no params, no result, encode
    ($input:ident, $fn:ident; @params []; @result_flag []; @encode_ret [$re:ty];) => {
        let result = $fn();
        $crate::__pvm_encode_return!(result : $re);
    };

    // no params, result, encode
    ($input:ident, $fn:ident; @params []; @result_flag [result]; @encode_ret [$re:ty];) => {
        match $fn() {
            Ok(result) => { $crate::__pvm_encode_return!(result : $re); }
            Err(e) => {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
            }
        }
    };

    // with params, no result, no encode
    ($input:ident, $fn:ident; @params [$($pn:ident : $pt:ty),+]; @result_flag []; @encode_ret [];) => {
        $crate::__pvm_decode_params!($input; 0; $($pn : $pt),+);
        $fn($($pn),+); return;
    };

    // with params, result, no encode
    ($input:ident, $fn:ident; @params [$($pn:ident : $pt:ty),+]; @result_flag [result]; @encode_ret [];) => {
        $crate::__pvm_decode_params!($input; 0; $($pn : $pt),+);
        match $fn($($pn),+) {
            Ok(()) => return,
            Err(e) => {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
            }
        }
    };

    // with params, no result, encode
    ($input:ident, $fn:ident; @params [$($pn:ident : $pt:ty),+]; @result_flag []; @encode_ret [$re:ty];) => {
        $crate::__pvm_decode_params!($input; 0; $($pn : $pt),+);
        let result = $fn($($pn),+);
        $crate::__pvm_encode_return!(result : $re);
    };

    // with params, result, encode
    ($input:ident, $fn:ident; @params [$($pn:ident : $pt:ty),+]; @result_flag [result]; @encode_ret [$re:ty];) => {
        $crate::__pvm_decode_params!($input; 0; $($pn : $pt),+);
        match $fn($($pn),+) {
            Ok(result) => { $crate::__pvm_encode_return!(result : $re); }
            Err(e) => {
                $crate::pallet_revive_uapi::HostFnImpl::return_value(
                    $crate::pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_decode_params {
    ($input:ident; $offset:expr;) => {};

    ($input:ident; $offset:expr; $name:ident : $ty:ty) => {
        let $name = <$ty as $crate::pvm_contract_types::SolDecode>::decode_at($input, $offset);
    };

    ($input:ident; $offset:expr; $name:ident : $ty:ty, $($rn:ident : $rt:ty),+) => {
        let $name = <$ty as $crate::pvm_contract_types::SolDecode>::decode_at($input, $offset);
        $crate::__pvm_decode_params!(
            $input;
            $offset + <$ty as $crate::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE;
            $($rn : $rt),+
        );
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __pvm_encode_return {
    ($result:ident : $ty:ty) => {{
        let mut __buf = [0u8; <$ty as $crate::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
        <$ty as $crate::pvm_contract_types::SolEncode>::encode_to(&$result, &mut __buf);
        $crate::pallet_revive_uapi::HostFnImpl::return_value(
            $crate::pallet_revive_uapi::ReturnFlags::empty(),
            &__buf,
        );
    }};
}
