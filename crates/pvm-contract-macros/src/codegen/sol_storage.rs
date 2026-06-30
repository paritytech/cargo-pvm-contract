//! `#[storage]` attribute macro: derive [`StorageComponent`] for a user struct
//! whose fields are themselves storage components (typically `Lazy<T>` and
//! `Mapping<K, V>`, but also nested `#[storage]` structs).
//!
//! Generated code is a thin shell over the same auto-numbering const chain
//! used by `#[contract]`:
//!
//! ```ignore
//! #[pvm_contract_sdk::storage]
//! pub struct Erc20 {
//!     total_supply: Lazy<U256>,
//!     balances: Mapping<Address, U256>,
//!     allowances: Mapping<Address, Mapping<Address, U256>>,
//! }
//! ```
//!
//! expands (roughly) to:
//!
//! ```ignore
//! pub struct Erc20 {
//!     total_supply: Lazy<U256>,
//!     balances: Mapping<Address, U256>,
//!     allowances: Mapping<Address, Mapping<Address, U256>>,
//! }
//!
//! impl ::pvm_contract_sdk::StorageComponent for Erc20 {
//!     const SLOTS: u64 =
//!           <Lazy<U256> as StorageComponent>::SLOTS
//!         + <Mapping<Address, U256> as StorageComponent>::SLOTS
//!         + <Mapping<Address, Mapping<Address, U256>> as StorageComponent>::SLOTS;
//!
//!     fn new_at(base: StorageKey, offset: u8, alone: bool, host: ::pvm_contract_sdk::Host) -> Self {
//!         // Per-field placement chain: each `LayoutStep` is computed by the
//!         // shared walker from the previous step plus this field's
//!         // PACKED_BYTES + SLOTS, so sub-word siblings pack solc-style.
//!         // `layout_step_component::<Ty>` is the StorageComponent-family
//!         // wrapper over the trait-agnostic `layout_step` primitive.
//!         const __pvm_storage_offset_total_supply: ::pvm_contract_sdk::LayoutStep =
//!             ::pvm_contract_sdk::layout_step_component::<Lazy<U256>>(
//!                 ::pvm_contract_sdk::LayoutStep::FIRST);
//!         const __pvm_storage_offset_balances: ::pvm_contract_sdk::LayoutStep =
//!             ::pvm_contract_sdk::layout_step_component::<Mapping<Address, U256>>(
//!                 __pvm_storage_offset_total_supply);
//!         const __pvm_storage_offset_allowances: ::pvm_contract_sdk::LayoutStep =
//!             ::pvm_contract_sdk::layout_step_component::<Mapping<Address, Mapping<Address, U256>>>(
//!                 __pvm_storage_offset_balances);
//!         // Per-field `alone` flag: true iff no neighbour shares the slot.
//!         const __pvm_storage_alone_total_supply: bool =
//!             true && __pvm_storage_offset_total_supply.slot != __pvm_storage_offset_balances.slot;
//!         const __pvm_storage_alone_balances: bool =
//!             __pvm_storage_offset_balances.slot != __pvm_storage_offset_total_supply.slot
//!             && __pvm_storage_offset_balances.slot != __pvm_storage_offset_allowances.slot;
//!         const __pvm_storage_alone_allowances: bool =
//!             __pvm_storage_offset_allowances.slot != __pvm_storage_offset_balances.slot && true;
//!         Erc20 {
//!             total_supply: <Lazy<U256> as StorageComponent>::new_at(
//!                 base.add(__pvm_storage_offset_total_supply.slot),
//!                 __pvm_storage_offset_total_supply.offset,
//!                 __pvm_storage_alone_total_supply, host.clone()),
//!             balances: <_ as StorageComponent>::new_at(
//!                 base.add(__pvm_storage_offset_balances.slot),
//!                 __pvm_storage_offset_balances.offset,
//!                 __pvm_storage_alone_balances, host.clone()),
//!             allowances: <_ as StorageComponent>::new_at(
//!                 base.add(__pvm_storage_offset_allowances.slot),
//!                 __pvm_storage_offset_allowances.offset,
//!                 __pvm_storage_alone_allowances, host.clone()),
//!         }
//!         // Every field — including the last — receives `host.clone()`.
//!         // `Host` is a ZST on riscv64 and a cheap `Rc` clone on host
//!         // targets, so cloning per-field has no measurable cost.
//!         // For this all-full-slot Erc20, each field lands on its own slot
//!         // (0, 1, 2) at offset 0 with `alone = true`; sub-word fields would
//!         // pack into shared slots with non-zero offsets and `alone = false`.
//!     }
//! }
//! ```
//!
//! Notes:
//! - Tuple and unit structs are rejected; only named-field structs make sense
//!   here because slot ordering is meaningful.
//! - `#[storage]` does NOT yet support pinning individual offsets via
//!   `#[slot(N)]`. The expectation is that embedded storage structs are
//!   declared in their natural field order; if the user wants a specific
//!   layout they can declare the leaf fields directly on the contract struct.
//! - The macro must be placed *before* any field uses the type, but doesn't
//!   need to be in the same module — the generated trait impl lives next to
//!   the struct, so it's visible wherever the struct is.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Fields, ItemStruct, Type};

use super::contract::find_derive_clone;
use super::sol_type::extract_field_info;
use super::storage_layout::{
    ChainField, alone_chain_consts, generate_layout_emit, slot_chain_consts,
};
use crate::signature::SolType;

/// Identifier prefix for the per-field offset `LayoutStep` consts. Defined
/// once and shared by the chain builder and every field-init/layout-emit
/// reference so the generated name and the references to it can't drift.
const OFFSET_PREFIX: &str = "__pvm_storage_offset_";
/// Identifier prefix for the per-field `alone` bool consts.
const ALONE_PREFIX: &str = "__pvm_storage_alone_";

pub fn expand_storage_struct(input: ItemStruct) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Same rationale as `#[contract]` storage structs: `#[storage]` components
    // are embedded in a contract storage tree where the borrow checker enforces
    // view-vs-mutating access (`&self` vs `&mut self`). A `Clone` impl would
    // let a view method clone the sub-component and obtain a fresh `&mut`,
    // bypassing the gate.
    if let Some(bad) = find_derive_clone(&input.attrs) {
        return Err(syn::Error::new_spanned(
            bad,
            "#[storage] structs must not derive `Clone`; the mutation gate \
             (`&self` vs `&mut self`) relies on the storage component being \
             `!Clone` to prevent view methods from smuggling out a `&mut`",
        ));
    }

    let named = match &input.fields {
        Fields::Named(named) => named,
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[storage] requires a struct with named fields. Unit and tuple structs are not supported.",
            ));
        }
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[storage] requires a struct with named fields. Tuple structs are not supported.",
            ));
        }
    };

    if named.named.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[storage] requires at least one storage field.",
        ));
    }

    // #[storage] fields are always auto-numbered (no `#[slot(N)]` support yet),
    // so a #[cfg]-disabled field would break the offset const chain *and*
    // silently shift every later field's slot. Reject up front; users who need
    // conditional fields should declare them directly on the contract struct
    // with `#[slot(N)]`.
    for f in &named.named {
        if let Some(cfg) = f.attrs.iter().find(|a| a.path().is_ident("cfg")) {
            return Err(syn::Error::new_spanned(
                cfg,
                "#[cfg] is not supported on #[storage] fields: it would shift \
                 the on-chain slot numbers of every field after it, producing a \
                 different storage layout per feature combination.",
            ));
        }
    }

    let field_names: Vec<&syn::Ident> = named
        .named
        .iter()
        .map(|f| f.ident.as_ref().expect("named fields"))
        .collect();
    let field_types: Vec<&syn::Type> = named.named.iter().map(|f| &f.ty).collect();

    // The SLOTS const sums every field's contribution.
    let slots_expr = {
        let terms: Vec<TokenStream> = field_types
            .iter()
            .map(|ty| quote! { <#ty as ::pvm_contract_sdk::StorageComponent>::SLOTS })
            .collect();
        quote! { #(#terms)+* }
    };

    // Per-field offset const chain (relative to base). Shared with `#[contract]`
    // via `slot_chain_consts` so both macros agree on the chain shape.
    //
    // `cfg_attrs: &[]` is safe here because the `#[cfg]`-on-field check above
    // rejects every cfg-attributed field with a compile error. If that
    // restriction is ever relaxed, this must be replaced with `&f.cfg_attrs`
    // so the chain consts are gated consistently with their fields.
    let chain_fields: Vec<ChainField<'_>> = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| ChainField {
            name,
            ty,
            cfg_attrs: &[],
        })
        .collect();
    let (offset_consts, offset_idents) = slot_chain_consts(OFFSET_PREFIX, &chain_fields);
    let alone_consts = alone_chain_consts(ALONE_PREFIX, &offset_idents, &chain_fields);

    let field_inits: Vec<TokenStream> = field_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ty = field_types[i];
            let const_ident = format_ident!("{}{}", OFFSET_PREFIX, name);
            let alone_ident = format_ident!("{}{}", ALONE_PREFIX, name);
            quote! {
                #name: <#ty as ::pvm_contract_sdk::StorageComponent>::new_at(
                    base.add(#const_ident.slot),
                    #const_ident.offset,
                    #alone_ident,
                    host.clone(),
                )
            }
        })
        .collect();

    // Per-field clear delegations: each field's own `clear` decides the
    // right thing (Lazy zeros its slot, Mapping is a no-op, nested
    // #[storage] recurses).
    let field_clears: Vec<TokenStream> = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| {
            quote! {
                <#ty as ::pvm_contract_sdk::StorageComponent>::clear(&mut self.#name);
            }
        })
        .collect();

    // Per-field layout-emit code: leaves push directly, embedded `#[storage]`
    // sub-structs recurse via `StorageLayoutEmit::emit_entries`.
    let layout_emits: Vec<TokenStream> = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| {
            let const_ident = format_ident!("{}{}", OFFSET_PREFIX, name);
            let slot_expr = quote! { base + #const_ident.slot };
            let offset_expr = quote! { #const_ident.offset };
            generate_layout_emit(
                &name.to_string(),
                ty,
                slot_expr,
                offset_expr,
                quote! { name_prefix },
            )
        })
        .collect();

    // Reuse the offset chain inside the layout helper. Each `emit_entries`
    // call rebuilds it locally so the consts stay in fn scope and the helper
    // doesn't have to track an external chain.
    let offset_consts_for_layout = offset_consts.clone();

    // The user's struct, unchanged.
    let user_struct = &input;

    Ok(quote! {
        #user_struct

        impl #impl_generics ::pvm_contract_sdk::StorageComponent
            for #struct_name #ty_generics
        #where_clause
        {
            const SLOTS: u64 = #slots_expr;

            // Embedded `#[storage]` sub-structs always start a fresh slot and
            // never pack with neighbouring contract fields. Matches solc —
            // packing applies inside the sub-struct, never across its outer
            // boundary.
            const PACKED_BYTES: usize = 32;

            fn new_at(
                base: ::pvm_contract_sdk::StorageKey,
                offset: u8,
                alone: bool,
                host: ::pvm_contract_sdk::Host,
            ) -> Self {
                debug_assert_eq!(offset, 0,
                    "#[storage] sub-struct always full-slot; offset must be 0");
                // Outer `alone` (from the embedding container) is irrelevant
                // here: this sub-struct's per-field `alone` is computed
                // internally from its own layout walker, since packing only
                // matters inside the sub-struct's own slot range.
                let _ = (offset, alone);
                #(#offset_consts)*
                #(#alone_consts)*
                #struct_name {
                    #(#field_inits),*
                }
            }

            /// Recursively clear every field. Matches solc's `delete
            /// struct_field` semantics — value-shaped fields (`Lazy<T>`)
            /// have their slots zeroed; sub-mapping fields are left intact
            /// (matches solc, since entries can't be enumerated); nested
            /// `#[storage]` sub-structs recurse.
            fn clear(&mut self) {
                #(#field_clears)*
            }
        }

        #[cfg(feature = "abi-gen")]
        impl #impl_generics ::pvm_contract_sdk::StorageLayoutEmit
            for #struct_name #ty_generics
        #where_clause
        {
            // `entries: &mut Vec<...>` matches the call shape in
            // `__storage_layout_json` and in the macro-generated recursion
            // code, which passes `entries` directly to reborrow.
            //
            // A `#[storage]` sub-struct always occupies a fresh slot
            // (`PACKED_BYTES == 32`), so the incoming `offset` is always `0`
            // here and is ignored; each field carries its own packed offset
            // via the per-field layout-step const chain below.
            fn emit_entries(
                base: u64,
                offset: u8,
                name_prefix: &str,
                entries: &mut ::std::vec::Vec<::pvm_contract_sdk::StorageLayoutEntry>,
            ) {
                let _ = offset;
                #(#offset_consts_for_layout)*
                #(#layout_emits)*
            }
        }

        // Name resolver for the layout-emit code path: when this struct is
        // used as the value type of a `Mapping<K, Self>`, the parent layout
        // emit asks `<Self as StorageTypeName>::name()` for the `"type"`
        // string of the `"mapping(K, …)"` entry. `pvm-contract-types` has no
        // blanket `StorageTypeName` impl — each type provides its own — so
        // `#[storage]` sub-structs need this explicit impl returning the
        // Rust ident.
        #[cfg(feature = "abi-gen")]
        impl #impl_generics ::pvm_contract_sdk::StorageTypeName
            for #struct_name #ty_generics
        #where_clause
        {
            fn name() -> ::std::string::String {
                ::std::string::String::from(#struct_name_str)
            }
        }
    })
}

// =========================================================================
// `#[derive(SolStorage)]` — emit StorageEncode/StorageDecode (+ Static*
// refinement for static structs) for a SolType-derived struct that the
// user wants to use as a `Lazy<S>` / `Mapping<_, S>` value.
//
// Separate from `#[derive(SolType)]` (which emits ABI traits) because:
// - Not every ABI struct belongs in storage (e.g. fn-param structs with
//   `Vec<T>` are fine for calldata, never go on-chain as storage values).
// - The "field unsupported in storage" case now emits a real
//   `compile_error!` at derive expansion (visible to `cargo check` and
//   `trybuild`) instead of a `const STORAGE_SLOTS = panic!(...)` stub that
//   only fires during MIR const-eval at `cargo build` time.
//
// Usage: `#[derive(SolType, SolStorage)] struct AddrAndCounter { ... }`
// for structs you want in both ABI and on-chain storage.
// =========================================================================

pub fn expand_sol_storage(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) | syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "SolStorage can only be derived for structs",
            ));
        }
    };

    let field_info = extract_field_info(fields)?;
    if field_info.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "SolStorage requires at least one field",
        ));
    }

    generate_sol_storage_impls(name, fields, &field_info)
}

/// How a field participates in the storage layout.
#[derive(Debug, Clone, Copy)]
enum StorageFieldKind {
    /// Packs into a parent slot at a sub-word offset.
    Packable,
    /// Solc-style dynamic field (`string` / `bytes`). Occupies one slot
    /// (header); body lives at `keccak256(slot) + i`.
    Dynamic,
    /// Not supported as a storage field — `compile_error!` is emitted by
    /// `generate_sol_storage_impls` when any field is `Unsupported`.
    Unsupported,
}

fn classify_storage_field(ty: &SolType) -> StorageFieldKind {
    match ty {
        SolType::Address
        | SolType::Bool
        | SolType::Uint(_)
        | SolType::Int(_)
        | SolType::Bytes(_) => StorageFieldKind::Packable,
        SolType::String | SolType::DynBytes => StorageFieldKind::Dynamic,
        // Custom types (nested structs) are not yet supported as a *packed
        // value* field of a `SolStorage` struct — that would need atomic
        // multi-field packed codegen which isn't implemented. This is an
        // optimization gap, NOT a solc-parity gap: a nested struct in storage
        // already works today (with byte-identical solc layout) via the
        // `#[storage]` attribute + `.view()/.view_mut()` composition path. The
        // rejection hint in `generate_sol_storage_impls` points users there.
        //
        // `Array<T>` (T != u8), `FixedArray`, `Tuple` in struct fields: deferred.
        SolType::Custom(_) | SolType::Array(_) | SolType::FixedArray(_, _) | SolType::Tuple(_) => {
            StorageFieldKind::Unsupported
        }
    }
}

fn get_field_types(fields: &Fields) -> Vec<&Type> {
    match fields {
        Fields::Named(named) => named.named.iter().map(|f| &f.ty).collect(),
        Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| &f.ty).collect(),
        Fields::Unit => Vec::new(),
    }
}

fn field_access_tokens(
    fields: &Fields,
    idx: usize,
    field_name: &Option<syn::Ident>,
) -> TokenStream {
    match fields {
        Fields::Named(_) => {
            let name = field_name.as_ref().expect("named field must have ident");
            quote! { self.#name }
        }
        Fields::Unnamed(_) => {
            let idx = syn::Index::from(idx);
            quote! { self.#idx }
        }
        Fields::Unit => quote! { compile_error!("storage struct cannot be a unit struct") },
    }
}

/// Emit the `StorageEncode` + `StorageDecode` impls for a SolStorage-derived
/// struct. Supports both static layouts (all fields `Packable`) and
/// dynamic-bodied layouts (fields include `String` / `Bytes` — solc-style
/// header-in-slot + body at `keccak256(slot) + i`). Fields classified as
/// `Unsupported` (nested SolType structs, tuples, fixed arrays of non-`u8`,
/// `Vec<T>` for `T != u8`) produce a `compile_error!`.
fn generate_sol_storage_impls(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    // Real compile error at derive expansion — visible to `cargo check` and
    // `trybuild`. Replaces the prior const-panic stub used when this code
    // lived inside `#[derive(SolType)]`.
    if let Some((field_idx, field_ty, unsupported_ty, is_nested_struct)) =
        field_info.iter().enumerate().find_map(|(idx, (_, ty))| {
            matches!(classify_storage_field(ty), StorageFieldKind::Unsupported).then(|| {
                (
                    idx,
                    get_field_types(fields)[idx],
                    ty.canonical_name(),
                    matches!(ty, SolType::Custom(_)),
                )
            })
        })
    {
        let field_label = match &field_info[field_idx].0 {
            Some(ident) => format!("field `{ident}`"),
            None => format!("field {field_idx}"),
        };
        // A nested struct *can* live in storage today — just not as a packed
        // value field. Point the user at the `#[storage]` + `.view()` path,
        // which yields the identical solc layout. Other unsupported kinds
        // (`Vec<T>`, tuples, fixed arrays) have no such workaround.
        let hint = if is_nested_struct {
            "Hint: a struct cannot yet be a packed value field of a `SolStorage` \
             struct. To store a nested struct, make BOTH structs \
             `#[pvm_contract_sdk::storage]` and access them through a field handle \
             or `Mapping<_, T>` with `.view()` / `.view_mut()` — this produces the \
             identical solc storage layout. (`#[derive(SolType)]` alone remains \
             correct if you only need ABI for calldata / events.)"
        } else {
            "Hint: `#[derive(SolType)]` alone still works — drop `SolStorage` if \
             you only need ABI (calldata / events)."
        };
        let msg = format!(
            "`{name}` cannot derive `SolStorage`: {field_label} has type \
             `{unsupported_ty}` (Rust: `{field_ty_str}`), which is not yet \
             supported as a `StorageEncode` field. Only fixed-size primitives \
             (`uint*`/`int*`/`address`/`bool`/`bytesN`), `string`, and `bytes` \
             (Rust `Bytes`) are supported today. {hint}",
            name = name,
            field_label = field_label,
            unsupported_ty = unsupported_ty,
            field_ty_str = quote!(#field_ty),
            hint = hint,
        );
        return Err(syn::Error::new_spanned(field_ty, msg));
    }

    let field_types: Vec<&Type> = get_field_types(fields);
    let n_fields = field_types.len();

    // ---- const layout walker (mirrors #[contract] / #[storage] auto-numbering) ----
    let walker_steps: Vec<TokenStream> = field_types
        .iter()
        .enumerate()
        .map(|(idx, ty)| {
            quote! {
                {
                    step = ::pvm_contract_sdk::layout_step_encode::<#ty>(step);
                    placements[#idx] = (step.slot as usize, step.offset as usize);
                }
            }
        })
        .collect();

    let kinds: Vec<StorageFieldKind> = field_info
        .iter()
        .map(|(_, ty)| classify_storage_field(ty))
        .collect();
    let has_dynamic = kinds.iter().any(|k| matches!(k, StorageFieldKind::Dynamic));

    let dynamic_field_flags: Vec<TokenStream> = kinds
        .iter()
        .map(|k| match k {
            StorageFieldKind::Dynamic => quote! { true },
            _ => quote! { false },
        })
        .collect();

    let layout_const = quote! {
        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const __STORAGE_LAYOUT: ([(usize, usize); #n_fields], usize, u64) = {
            let mut placements: [(usize, usize); #n_fields] = [(0, 0); #n_fields];
            let mut step: ::pvm_contract_sdk::LayoutStep =
                ::pvm_contract_sdk::LayoutStep::FIRST;
            #(#walker_steps)*
            let total = if #n_fields == 0 { 0 } else { step.next_slot as usize + 1 };

            let dynamic_field_flags: [bool; #n_fields] = [#(#dynamic_field_flags),*];
            let mut dynamic_mask: u64 = 0;
            let mut __mi: usize = 0;
            while __mi < #n_fields {
                if dynamic_field_flags[__mi] {
                    let s = placements[__mi].0;
                    assert!(
                        s < 64,
                        "`#[derive(SolStorage)]`: dynamic field at slot >= 64 \
                         exceeds the u64 `dynamic_mask` bitmask.",
                    );
                    dynamic_mask |= 1u64 << s;
                }
                __mi += 1;
            }
            (placements, total, dynamic_mask)
        };
    };

    // ---- per-slot static encoder body ----
    let mut encode_arms = Vec::with_capacity(n_fields);
    for (i, (field_name, _)) in field_info.iter().enumerate() {
        let field_ty = field_types[i];
        let field_access = field_access_tokens(fields, i, field_name);
        let arm = match kinds[i] {
            StorageFieldKind::Packable => quote! {
                {
                    let (s, o) = Self::__STORAGE_LAYOUT.0[#i];
                    if s == slot_idx {
                        <#field_ty as ::pvm_contract_sdk::StoragePackable>::pack_into(
                            &#field_access, buf, o,
                        );
                    }
                }
            },
            StorageFieldKind::Dynamic => quote! {
                // Dynamic fields don't pack into the parent's slot buffer.
                {}
            },
            StorageFieldKind::Unsupported => unreachable!("rejected above"),
        };
        encode_arms.push(arm);
    }

    // ---- slot-buffer decoder body (static structs only) ----
    let decode_construct = if has_dynamic {
        quote! {}
    } else {
        let mk_field_decode = |idx: usize, field_ty: &Type| -> TokenStream {
            quote! {
                {
                    let (s, o) = Self::__STORAGE_LAYOUT.0[#idx];
                    <#field_ty as ::pvm_contract_sdk::StoragePackable>::unpack_from(
                        &slots[s], o,
                    )
                }
            }
        };
        match fields {
            Fields::Named(named) => {
                let mut field_lets = Vec::new();
                for (i, field) in named.named.iter().enumerate() {
                    debug_assert!(matches!(kinds[i], StorageFieldKind::Packable));
                    let field_name = field.ident.as_ref().unwrap();
                    let field_ty = &field.ty;
                    let value_expr = mk_field_decode(i, field_ty);
                    field_lets.push(quote! { #field_name: #value_expr });
                }
                quote! { Self { #(#field_lets),* } }
            }
            Fields::Unnamed(unnamed) => {
                let mut field_exprs = Vec::new();
                for (i, field) in unnamed.unnamed.iter().enumerate() {
                    debug_assert!(matches!(kinds[i], StorageFieldKind::Packable));
                    let field_ty = &field.ty;
                    let value_expr = mk_field_decode(i, field_ty);
                    field_exprs.push(value_expr);
                }
                quote! { Self(#(#field_exprs),*) }
            }
            Fields::Unit => quote! { Self },
        }
    };

    // Module-scope `const _: () = ...` assertion. Evaluated at type-check
    // time (cargo check), so trybuild UI fixtures can pin the rejection
    // without needing a use site to force monomorphization. Each dynamic
    // struct emits exactly one of these; the inline trait-method bodies
    // no longer carry their own per-method copy.
    // Embed the concrete limit in the message (read from the source of truth
    // so it can't drift) alongside the symbolic `MAX_STATIC_SLOTS` name.
    let max_static_slots =
        proc_macro2::Literal::usize_unsuffixed(::pvm_contract_types::MAX_STATIC_SLOTS);
    let slot_count_assert_item = quote! {
        #[doc(hidden)]
        const _: () = {
            assert!(
                <#name as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS
                    <= ::pvm_contract_sdk::MAX_STATIC_SLOTS,
                concat!(
                    "`#[derive(SolStorage)]` on `",
                    stringify!(#name),
                    "`: a storage struct cannot exceed ",
                    stringify!(#max_static_slots),
                    " storage slots (MAX_STATIC_SLOTS). Reduce the field count.",
                ),
            );
        };
    };

    if has_dynamic {
        // Dynamic struct: custom write/clear/read/try_read with per-field
        // static/dynamic split.
        let mut write_calls: Vec<TokenStream> = Vec::new();
        let mut clear_calls: Vec<TokenStream> = Vec::new();
        let mut read_field_lets: Vec<TokenStream> = Vec::new();
        let mut read_construct_fields: Vec<TokenStream> = Vec::new();

        for (i, (field_name, _)) in field_info.iter().enumerate() {
            let field_ty = field_types[i];
            let field_access = field_access_tokens(fields, i, field_name);
            if let StorageFieldKind::Dynamic = kinds[i] {
                write_calls.push(quote! {
                    {
                        let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                        let mut sub_key = *base_key;
                        for _ in 0..s { ::pvm_contract_sdk::__private::inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageEncode>::write_to_storage(
                            &#field_access, host, &sub_key,
                        );
                    }
                });
                clear_calls.push(quote! {
                    {
                        let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                        let mut sub_key = *base_key;
                        for _ in 0..s { ::pvm_contract_sdk::__private::inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageEncode>::clear_storage(
                            host, &sub_key,
                        );
                    }
                });
            }

            let local = quote::format_ident!("__field_{}", i);
            let read_expr = match kinds[i] {
                StorageFieldKind::Packable => quote! {
                    {
                        let (s, o) = Self::__STORAGE_LAYOUT.0[#i];
                        <#field_ty as ::pvm_contract_sdk::StoragePackable>::unpack_from(
                            &__slots[s], o,
                        )
                    }
                },
                StorageFieldKind::Dynamic => quote! {
                    {
                        let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                        let mut sub_key = *base_key;
                        for _ in 0..s { ::pvm_contract_sdk::__private::inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageDecode>::read_from_storage(
                            host, &sub_key,
                        )
                    }
                },
                StorageFieldKind::Unsupported => unreachable!(),
            };
            read_field_lets.push(quote! { let #local = #read_expr; });
            match fields {
                Fields::Named(named) => {
                    let fname = named.named[i].ident.as_ref().unwrap();
                    read_construct_fields.push(quote! { #fname: #local });
                }
                Fields::Unnamed(_) => {
                    read_construct_fields.push(quote! { #local });
                }
                Fields::Unit => {}
            }
        }

        let read_construct = match fields {
            Fields::Named(_) => quote! { Self { #(#read_construct_fields),* } },
            Fields::Unnamed(_) => quote! { Self(#(#read_construct_fields),*) },
            Fields::Unit => quote! { Self },
        };

        Ok(quote! {
            // Eager type-check-time slot-count guard. Module-scope so it
            // fires under `cargo check` (visible to trybuild) instead of
            // only firing when a trait method is monomorphized.
            #slot_count_assert_item

            impl #name {
                #layout_const

                #[doc(hidden)]
                #[inline]
                fn __encode_static_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
                    *buf = [0u8; 32];
                    #(#encode_arms)*
                }
            }

            impl ::pvm_contract_sdk::StorageEncode for #name {
                const STORAGE_SLOTS: usize = Self::__STORAGE_LAYOUT.1;
                const PACKED_BYTES: usize = 32;
                const HAS_DYNAMIC_BODY: bool = true;

                fn write_to_storage(
                    &self,
                    host: &::pvm_contract_sdk::Host,
                    base_key: &[u8; 32],
                ) {
                    let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                    let __n = <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS;
                    debug_assert!(__n <= ::pvm_contract_sdk::MAX_STATIC_SLOTS);
                    let mut __slots = [[0u8; 32]; ::pvm_contract_sdk::MAX_STATIC_SLOTS];
                    for __i in 0..__n {
                        if dynamic_mask & (1u64 << __i) == 0 {
                            Self::__encode_static_slot(self, __i, &mut __slots[__i]);
                        }
                    }
                    ::pvm_contract_sdk::__private::write_static_slots(
                        host, base_key, &__slots[..__n], dynamic_mask,
                    );
                    #(#write_calls)*
                }

                fn clear_storage(
                    host: &::pvm_contract_sdk::Host,
                    base_key: &[u8; 32],
                ) {
                    let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                    let __n = <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS;
                    ::pvm_contract_sdk::__private::clear_static_slots(
                        host, base_key, __n, dynamic_mask,
                    );
                    #(#clear_calls)*
                }
            }

            impl ::pvm_contract_sdk::StorageDecode for #name {
                fn read_from_storage(
                    host: &::pvm_contract_sdk::Host,
                    base_key: &[u8; 32],
                ) -> Self {
                    let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                    let __n = <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS;
                    debug_assert!(__n <= ::pvm_contract_sdk::MAX_STATIC_SLOTS);
                    let mut __slots = [[0u8; 32]; ::pvm_contract_sdk::MAX_STATIC_SLOTS];
                    ::pvm_contract_sdk::__private::load_static_slots(
                        host, base_key, __n, dynamic_mask, &mut __slots,
                    );

                    #(#read_field_lets)*

                    #read_construct
                }

                fn try_read_from_storage(
                    host: &::pvm_contract_sdk::Host,
                    base_key: &[u8; 32],
                ) -> Option<Self> {
                    let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                    let __n = <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS;
                    debug_assert!(__n <= ::pvm_contract_sdk::MAX_STATIC_SLOTS);
                    let mut __slots = [[0u8; 32]; ::pvm_contract_sdk::MAX_STATIC_SLOTS];
                    // Single SLOAD pass: presence check across all slots +
                    // static-slot load reused by `#read_field_lets` below.
                    let __any = ::pvm_contract_sdk::__private::try_load_static_slots(
                        host, base_key, __n, dynamic_mask, &mut __slots,
                    );
                    if !__any {
                        return None;
                    }

                    #(#read_field_lets)*

                    Some(#read_construct)
                }
            }

            // Storage-layout JSON type-name resolver — same shape as the
            // static branch and as `#[storage]` sub-structs. Returns the
            // Rust ident so storage layout JSON uses solc-struct-style
            // names, not the ABI tuple notation `SolEncode::SOL_NAME` gives.
            #[cfg(feature = "abi-gen")]
            impl ::pvm_contract_sdk::StorageTypeName for #name {
                fn name() -> ::std::string::String {
                    ::std::string::String::from(stringify!(#name))
                }
            }
        })
    } else {
        // Static struct: universal trait methods delegate to shared helpers;
        // encode_slot + from_slots live on Static* refinement.
        Ok(quote! {
            // Eager type-check-time slot-count guard, same as the dynamic
            // branch — fires under `cargo check` instead of only when a trait
            // method is monomorphized.
            #slot_count_assert_item

            impl #name {
                #layout_const
            }

            impl ::pvm_contract_sdk::StorageEncode for #name {
                const STORAGE_SLOTS: usize = Self::__STORAGE_LAYOUT.1;
                const PACKED_BYTES: usize = 32;

                #[inline]
                fn write_to_storage(
                    &self,
                    host: &::pvm_contract_sdk::Host,
                    key: &[u8; 32],
                ) {
                    <Self as ::pvm_contract_sdk::StaticStorageEncode>::write_to_storage_static(self, host, key)
                }

                #[inline]
                fn clear_storage(
                    host: &::pvm_contract_sdk::Host,
                    key: &[u8; 32],
                ) {
                    <Self as ::pvm_contract_sdk::StaticStorageEncode>::clear_storage_static(host, key)
                }
            }

            impl ::pvm_contract_sdk::StorageDecode for #name {
                #[inline]
                fn read_from_storage(
                    host: &::pvm_contract_sdk::Host,
                    key: &[u8; 32],
                ) -> Self {
                    <Self as ::pvm_contract_sdk::StaticStorageDecode>::read_from_storage_static(host, key)
                }

                #[inline]
                fn try_read_from_storage(
                    host: &::pvm_contract_sdk::Host,
                    key: &[u8; 32],
                ) -> Option<Self> {
                    <Self as ::pvm_contract_sdk::StaticStorageDecode>::try_read_from_storage_static(host, key)
                }
            }

            impl ::pvm_contract_sdk::StaticStorageEncode for #name {
                fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
                    *buf = [0u8; 32];
                    #(#encode_arms)*
                }
            }

            impl ::pvm_contract_sdk::StaticStorageDecode for #name {
                fn from_slots(slots: &[[u8; 32]]) -> Self {
                    #decode_construct
                }
            }

            // Auto-opt into `StorageArrayElement` for static structs so users
            // can write `Lazy<[MyStruct; N]>` / `StorageVec<[MyStruct; N]>` /
            // `Mapping<K, [MyStruct; N]>` without an extra manual impl. Only
            // the static branch reaches here; dynamic-body structs (which
            // can't satisfy the `StaticStorageEncode + StaticStorageDecode`
            // supertrait bound) are excluded by construction.
            impl ::pvm_contract_sdk::StorageArrayElement for #name {}

            // Storage-layout JSON type-name resolver. When this struct is
            // used as the value of a `Lazy<Self>` or `Mapping<_, Self>`,
            // the layout-emit code calls `<Self as StorageTypeName>::name()`
            // for the `"type"` field. Emitting the Rust ident here keeps
            // the name parallel with `#[storage]` sub-structs (which also
            // emit their ident) — both code paths produce
            // solc-struct-style names rather than the ABI tuple notation
            // that `SolEncode::SOL_NAME` would supply.
            #[cfg(feature = "abi-gen")]
            impl ::pvm_contract_sdk::StorageTypeName for #name {
                fn name() -> ::std::string::String {
                    ::std::string::String::from(stringify!(#name))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ItemStruct {
        syn::parse_str(src).expect("input parses")
    }

    #[test]
    fn generates_storage_component_impl() {
        let input = parse(
            r#"
            pub struct Erc20 {
                total_supply: Lazy<U256>,
                balances: Mapping<Address, U256>,
            }
        "#,
        );
        let output = expand_storage_struct(input).unwrap().to_string();

        // The original struct is preserved.
        assert!(
            output.contains("pub struct Erc20"),
            "struct should be preserved: {output}"
        );

        // SLOTS sums each field's SLOTS.
        assert!(
            output.contains("const SLOTS : u64 = < Lazy < U256 > as :: pvm_contract_sdk :: StorageComponent > :: SLOTS + < Mapping < Address , U256 > as :: pvm_contract_sdk :: StorageComponent > :: SLOTS"),
            "SLOTS const should sum field SLOTS. Got: {output}"
        );

        // First field seeds from LayoutStep::FIRST via layout_step.
        assert!(
            output.contains(
                "const __pvm_storage_offset_total_supply : :: pvm_contract_sdk :: LayoutStep = :: pvm_contract_sdk :: layout_step_component :: < Lazy < U256 > > (:: pvm_contract_sdk :: LayoutStep :: FIRST)"
            ),
            "first offset should seed from LayoutStep::FIRST: {output}"
        );

        // Each field's slot is base.add(step.slot), offset is step.offset.
        assert!(
            output.contains("base . add (__pvm_storage_offset_total_supply . slot)"),
            "field init should derive its key from base.add(step.slot): {output}"
        );
    }

    #[test]
    fn rejects_tuple_struct() {
        let input = parse("pub struct T(u32, u32);");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(
            err.contains("Tuple structs are not supported"),
            "Got: {err}"
        );
    }

    #[test]
    fn rejects_unit_struct() {
        let input = parse("pub struct U;");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(
            err.contains("Unit and tuple structs are not supported"),
            "Got: {err}"
        );
    }

    #[test]
    fn rejects_empty_named_struct() {
        let input = parse("pub struct E {}");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("at least one storage field"), "Got: {err}");
    }

    #[test]
    fn rejects_derive_clone() {
        let input = parse(
            r#"
            #[derive(Clone)]
            pub struct S {
                a: Lazy<U256>,
            }
        "#,
        );
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("must not derive `Clone`"), "Got: {err}");
    }

    #[test]
    fn rejects_cfg_on_storage_field() {
        let input = parse(
            r#"
            pub struct S {
                #[cfg(feature = "extra")]
                a: Lazy<U256>,
                b: Lazy<U256>,
            }
        "#,
        );
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("#[cfg] is not supported"), "Got: {err}");
    }

    #[test]
    fn supports_generics() {
        let input = parse(
            r#"
            pub struct Container<T> {
                value: Lazy<T>,
            }
        "#,
        );
        let output = expand_storage_struct(input).unwrap().to_string();
        // The impl picks up the generics.
        assert!(
            output
                .contains("impl < T > :: pvm_contract_sdk :: StorageComponent for Container < T >"),
            "should propagate generics: {output}"
        );
    }
}
