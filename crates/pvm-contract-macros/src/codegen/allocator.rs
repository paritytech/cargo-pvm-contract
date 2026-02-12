use proc_macro2::TokenStream;
use quote::quote;

/// Default heap size: 256 KiB
const DEFAULT_HEAP_SIZE: usize = 262144;

/// Generate the `#[global_allocator]` picoalloc setup.
/// Only emitted for the entry-point crate to avoid linker conflicts.
pub fn generate_allocator(heap_size: Option<usize>) -> TokenStream {
    let size = heap_size.unwrap_or(DEFAULT_HEAP_SIZE);
    quote! {
        #[global_allocator]
        static mut __PVM_ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<#size>>> = {
            static mut ARRAY: picoalloc::Array<#size> = picoalloc::Array([0u8; #size]);
            picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
                picoalloc::ArrayPointer::new(&raw mut ARRAY)
            }))
        };
    }
}
