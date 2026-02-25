//! A simple bump allocator for PVM smart contracts.
//!
//! This allocator is designed for use in `no_std` environments where memory is never freed
//! (e.g., short-lived smart contract executions). It simply bumps a pointer forward for each
//! allocation and never reclaims memory on `dealloc`.
//!
//! Adapted for the PVM contract toolchain.
//!
//! # Usage
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: pvm_bump_allocator::BumpAllocator<{ 1024 * 1024 }> =
//!     pvm_bump_allocator::BumpAllocator::new();
//! ```

#![no_std]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::{Cell, UnsafeCell};

/// A bump allocator backed by a fixed-size heap.
///
/// `HEAP_SIZE` is the total number of bytes available for allocation.
/// Memory is never freed — `dealloc` is a no-op.
pub struct BumpAllocator<const HEAP_SIZE: usize> {
    offset: Cell<usize>,
    heap: UnsafeCell<[u8; HEAP_SIZE]>,
}

// SAFETY: PVM contracts are single-threaded. The `Sync` bound is required by
// `GlobalAlloc` (the allocator must live in a `static`), but no concurrent
// access actually occurs.
unsafe impl<const HEAP_SIZE: usize> Sync for BumpAllocator<HEAP_SIZE> {}

impl<const HEAP_SIZE: usize> Default for BumpAllocator<HEAP_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const HEAP_SIZE: usize> BumpAllocator<HEAP_SIZE> {
    /// Creates a new bump allocator with a zeroed heap of `HEAP_SIZE` bytes.
    pub const fn new() -> Self {
        Self {
            offset: Cell::new(0),
            heap: UnsafeCell::new([0u8; HEAP_SIZE]),
        }
    }
}

unsafe impl<const HEAP_SIZE: usize> GlobalAlloc for BumpAllocator<HEAP_SIZE> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        let current = self.offset.get();
        let aligned = (current + align - 1) & !(align - 1);
        let Some(next) = aligned.checked_add(size) else {
            core::panic!("exhausted heap limit");
        };

        if next > HEAP_SIZE {
            core::panic!("exhausted heap limit");
        }

        self.offset.set(next);
        let heap_ptr = self.heap.get() as *mut u8;
        unsafe { heap_ptr.add(aligned) }
    }

    // The heap is zero-initialized and memory is never reused, so every
    // region returned by `alloc` is already zeroed.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { self.alloc(layout) }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    #[test]
    fn alloc_single_byte() {
        let alloc = BumpAllocator::<1024>::new();
        let layout = Layout::from_size_align(1, 1).unwrap();
        let ptr = unsafe { alloc.alloc(layout) };
        assert!(!ptr.is_null());
    }

    #[test]
    fn alloc_aligned() {
        let alloc = BumpAllocator::<1024>::new();

        // Allocate 1 byte to misalign the offset
        let layout1 = Layout::from_size_align(1, 1).unwrap();
        unsafe { alloc.alloc(layout1) };

        // Next allocation with 8-byte alignment must be properly aligned
        let layout2 = Layout::from_size_align(8, 8).unwrap();
        let ptr = unsafe { alloc.alloc(layout2) };
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 8, 0);
    }

    #[test]
    fn alloc_fills_exactly() {
        let alloc = BumpAllocator::<64>::new();
        let layout = Layout::from_size_align(64, 1).unwrap();
        assert!(!unsafe { alloc.alloc(layout) }.is_null());
    }

    #[test]
    #[should_panic = "exhausted heap limit"]
    fn alloc_panics_when_full() {
        let alloc = BumpAllocator::<64>::new();
        unsafe {
            alloc.alloc(Layout::from_size_align(64, 1).unwrap());
            alloc.alloc(Layout::from_size_align(1, 1).unwrap());
        }
    }

    #[test]
    #[should_panic = "exhausted heap limit"]
    fn alloc_oom_panics() {
        let alloc = BumpAllocator::<16>::new();
        unsafe { alloc.alloc(Layout::from_size_align(17, 1).unwrap()) };
    }

    #[test]
    #[should_panic = "exhausted heap limit"]
    fn alloc_oom_due_to_alignment_padding() {
        // 9 bytes of heap: alloc 1 byte, then try 8 bytes with align 8
        // offset=1, aligned=8, 8+8=16 > 9 → OOM
        let alloc = BumpAllocator::<9>::new();
        unsafe {
            alloc.alloc(Layout::from_size_align(1, 1).unwrap());
            alloc.alloc(Layout::from_size_align(8, 8).unwrap());
        }
    }

    #[test]
    fn multiple_allocations_dont_overlap() {
        let alloc = BumpAllocator::<1024>::new();
        let layout = Layout::from_size_align(32, 8).unwrap();

        let p1 = unsafe { alloc.alloc(layout) };
        let p2 = unsafe { alloc.alloc(layout) };
        let p3 = unsafe { alloc.alloc(layout) };
        assert!(!p1.is_null() && !p2.is_null() && !p3.is_null());

        // Bump allocator returns sequential non-overlapping regions
        assert!(p1 as usize + 32 <= p2 as usize);
        assert!(p2 as usize + 32 <= p3 as usize);
    }

    #[test]
    fn write_and_read_back() {
        let alloc = BumpAllocator::<256>::new();
        let ptr = unsafe { alloc.alloc(Layout::from_size_align(4, 4).unwrap()) } as *mut u32;
        assert!(!ptr.is_null());
        unsafe {
            ptr.write(0xDEAD_BEEF);
            assert_eq!(ptr.read(), 0xDEAD_BEEF);
        }
    }

    #[test]
    fn no_out_of_bounds_writes() {
        #[repr(C)]
        struct Guarded {
            alloc: BumpAllocator<64>,
            sentinel: [u8; 64],
        }

        let guarded = Guarded {
            alloc: BumpAllocator::new(),
            sentinel: [0xAA; 64],
        };

        let ptr = unsafe { guarded.alloc.alloc(Layout::from_size_align(64, 1).unwrap()) };
        assert!(!ptr.is_null());

        for i in 0..64 {
            unsafe { ptr.add(i).write(0xFF) };
        }

        for byte in &guarded.sentinel {
            assert_eq!(*byte, 0xAA, "out-of-bounds write detected");
        }
    }
}
