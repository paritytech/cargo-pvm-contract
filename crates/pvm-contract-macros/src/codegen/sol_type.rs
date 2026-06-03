use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields, Type};

use crate::signature::SolType;

pub fn expand_sol_type(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolType can only be derived for structs",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolType can only be derived for structs",
            ));
        }
    };

    let field_info = extract_field_info(fields)?;

    if field_info.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "SolType requires at least one field",
        ));
    }

    // Unresolved custom types cannot be queried via SolType::is_dynamic; route
    // through dynamic codegen, which now uses trait-based runtime/static checks.
    let has_dynamic = field_info
        .iter()
        .any(|(_, t)| t.has_custom_types() || t.is_dynamic() == Some(true));
    if has_dynamic {
        expand_dynamic_sol_type(name, fields, &field_info)
    } else {
        expand_static_sol_type(name, fields, &field_info)
    }
}

fn expand_static_sol_type(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    let sol_name_expr = build_sol_name_expr(field_info);
    let total_size_expr = build_total_size_expr(field_info);
    let encode_body = generate_static_encode_body(fields);
    let decode_body = generate_static_decode_body(fields, false);
    let decode_body_unchecked = generate_static_decode_body(fields, true);

    let storage_impls = generate_storage_impls(name, fields, field_info)?;

    #[cfg(feature = "abi-gen")]
    let abi_param_fn = generate_abi_param_fn(fields, field_info);
    #[cfg(not(feature = "abi-gen"))]
    let abi_param_fn = quote::quote! {};

    Ok(quote! {
        impl ::pvm_contract_sdk::SolEncode for #name {
            const IS_DYNAMIC: bool = false;
            const SOL_NAME: &'static str = #sol_name_expr;
            const HEAD_SIZE: usize = #total_size_expr;

            #[inline]
            fn encode_body_len(&self) -> usize {
                #total_size_expr
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            /// Indexed topic for a struct value is `keccak256(abi.encode(self))`
            /// per the Solidity event spec, not the right-aligned default.
            fn indexed_topic(&self) -> [u8; 32] {
                const __ENC_SIZE: usize = #total_size_expr;
                let mut __buf = [0u8; __ENC_SIZE];
                <Self as ::pvm_contract_sdk::SolEncode>::encode_to(self, &mut __buf);
                ::pvm_contract_sdk::keccak256(&__buf)
            }

            #abi_param_fn
        }

        impl ::pvm_contract_sdk::StaticEncodedLen for #name {
            const ENCODED_SIZE: usize = #total_size_expr;
        }

        impl ::pvm_contract_sdk::StaticDecode for #name {
            unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self  {
                #decode_body_unchecked
            }
        }

        impl ::pvm_contract_sdk::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Result<Self, ::pvm_contract_sdk::DecodeError>  {
                #decode_body
            }
        }

        impl ::pvm_contract_sdk::SolArrayElement for #name {}

        #storage_impls
    })
}

fn expand_dynamic_sol_type(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    let sol_name_expr = build_sol_name_expr(field_info);
    let is_dynamic_expr = build_is_dynamic_expr(fields, field_info);
    let head_size_expr = build_dynamic_head_size_expr(fields, field_info);
    let encode_len_body = generate_dynamic_encode_len(fields, field_info, &head_size_expr);
    let encode_body = generate_dynamic_encode_body(fields, field_info, &head_size_expr);
    let decode_body = generate_dynamic_decode_body(fields, field_info);

    // Emit storage impls when every field is storage-compatible (static or
    // custom). Truly dynamic fields (`Vec`, `String`, etc.) return an error
    // from `generate_storage_impls`; we silently drop the storage impls in
    // that case so the struct remains usable for ABI / calldata even though
    // it can't be a `Mapping<K, V>` or `Lazy<T>` value.
    let storage_impls = match generate_storage_impls(name, fields, field_info) {
        Ok(ts) => ts,
        Err(_) => quote! {},
    };

    #[cfg(feature = "abi-gen")]
    let abi_param_fn = generate_abi_param_fn(fields, field_info);
    #[cfg(not(feature = "abi-gen"))]
    let abi_param_fn = quote::quote! {};

    Ok(quote! {
        impl ::pvm_contract_sdk::SolEncode for #name {
            const IS_DYNAMIC: bool = #is_dynamic_expr;
            const SOL_NAME: &'static str = #sol_name_expr;
            const HEAD_SIZE: usize = #head_size_expr;

            fn encode_body_len(&self) -> usize {
                #encode_len_body
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            #abi_param_fn
        }

        impl ::pvm_contract_sdk::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Result<Self, ::pvm_contract_sdk::DecodeError> {
                #decode_body
            }

            fn decode_tail(input: &[u8], offset: usize) -> Result<Self, ::pvm_contract_sdk::DecodeError>  {
                Self::decode_at(input, offset)
            }
        }

        impl ::pvm_contract_sdk::SolArrayElement for #name {}

        #storage_impls
    })
}

/// Generate the `abi_param()` method override for a struct.
/// Returns `"type": "tuple"` with `components` listing each field.
#[cfg(feature = "abi-gen")]
fn generate_abi_param_fn(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);

    let component_exprs: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((field_name, _), field_ty)| {
            let name_str = match field_name {
                Some(ident) => ident.to_string(),
                None => String::new(),
            };
            quote! {
                <#field_ty as ::pvm_contract_sdk::SolEncode>::abi_param(#name_str)
            }
        })
        .collect();

    quote! {
        fn abi_param(name: &str) -> ::pvm_contract_sdk::AbiParam {
            extern crate alloc;
            ::pvm_contract_sdk::AbiParam {
                name: alloc::string::String::from(name),
                param_type: alloc::string::String::from("tuple"),
                components: alloc::vec![#(#component_exprs),*],
            }
        }
    }
}

fn build_is_dynamic_expr(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());
    if !has_custom {
        let is_dynamic = field_info.iter().any(|(_, t)| t.is_dynamic() == Some(true));
        return quote! { #is_dynamic };
    }

    let field_types = get_field_types(fields);
    let parts: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((_, t), ty)| match t.is_dynamic() {
            Some(is_dyn) => quote! { #is_dyn },
            None => quote! { <#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC },
        })
        .collect();

    quote! { false #(|| #parts)* }
}

pub(crate) fn sol_type_name_parts(ty: &SolType, parts: &mut Vec<TokenStream>) {
    match ty {
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                parts.push(quote! { <#type_path as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
            }
            Err(err) => {
                let msg =
                    format!("Invalid custom type path `{name}` in `#[derive(SolType)]`: {err}");
                parts.push(quote! { compile_error!(#msg) });
            }
        },
        SolType::Array(inner) if inner.has_custom_types() => {
            sol_type_name_parts(inner, parts);
            parts.push(quote! { "[]" });
        }
        SolType::FixedArray(inner, size) if inner.has_custom_types() => {
            sol_type_name_parts(inner, parts);
            let suffix = format!("[{}]", size);
            parts.push(quote! { #suffix });
        }
        SolType::Tuple(types) if types.iter().any(|t| t.has_custom_types()) => {
            parts.push(quote! { "(" });
            for (i, t) in types.iter().enumerate() {
                if i > 0 {
                    parts.push(quote! { "," });
                }
                sol_type_name_parts(t, parts);
            }
            parts.push(quote! { ")" });
        }
        _ => {
            let name = ty.canonical_name();
            parts.push(quote! { #name });
        }
    }
}

fn build_sol_signature(field_info: &[(Option<syn::Ident>, SolType)]) -> String {
    let field_types = field_info
        .iter()
        .map(|(_, sol_type)| sol_type.canonical_name())
        .collect::<Vec<_>>();
    format!("({})", field_types.join(","))
}

fn build_sol_name_expr(field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let sig = build_sol_signature(field_info);
        return quote! { #sig };
    }

    let mut parts: Vec<TokenStream> = Vec::new();
    parts.push(quote! { "(" });

    for (i, (_, sol_type)) in field_info.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        sol_type_name_parts(sol_type, &mut parts);
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}

fn build_total_size_expr(field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let total: usize = field_info
            .iter()
            .map(|(_, t)| {
                t.head_size()
                    .expect("build_total_size_expr called on unresolved custom type")
            })
            .sum();
        return quote! { #total };
    }

    let size_exprs: Vec<TokenStream> = field_info
        .iter()
        .map(|(_, sol_type)| sol_type_head_size_expr(sol_type))
        .collect();

    quote! { 0 #(+ #size_exprs)* }
}

fn sol_type_head_size_expr(ty: &SolType) -> TokenStream {
    match ty {
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                quote! { <#type_path as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE }
            }
            Err(err) => {
                let msg =
                    format!("Invalid custom type path `{name}` in `#[derive(SolType)]`: {err}");
                quote! {{
                    compile_error!(#msg);
                    0usize
                }}
            }
        },
        SolType::FixedArray(inner, size) if inner.has_custom_types() => {
            let inner_size = sol_type_head_size_expr(inner);
            let size_lit = *size;
            quote! { (#inner_size * #size_lit) }
        }
        SolType::Tuple(types) if types.iter().any(|t| t.has_custom_types()) => {
            let parts: Vec<TokenStream> = types.iter().map(sol_type_head_size_expr).collect();
            quote! { (0 #(+ #parts)*) }
        }
        _ => {
            let size = ty
                .head_size()
                .expect("sol_type_head_size_expr called on unresolved custom type");
            quote! { #size }
        }
    }
}

fn get_field_types(fields: &Fields) -> Vec<&Type> {
    match fields {
        Fields::Named(named) => named.named.iter().map(|f| &f.ty).collect(),
        Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| &f.ty).collect(),
        Fields::Unit => vec![],
    }
}

// -----------------------------------------------------------------------
// Static struct encode/decode — always uses trait-based dispatch
// -----------------------------------------------------------------------

fn generate_static_encode_body(fields: &Fields) -> TokenStream {
    let field_types = get_field_types(fields);
    let mut stmts = Vec::new();
    stmts.push(quote! { let mut __offset: usize = 0; });

    for (i, field_ty) in field_types.iter().enumerate() {
        let field_access = match fields {
            Fields::Named(named) => {
                let name = named.named[i].ident.as_ref().unwrap();
                quote! { self.#name }
            }
            Fields::Unnamed(_) => {
                let idx = syn::Index::from(i);
                quote! { self.#idx }
            }
            Fields::Unit => continue,
        };

        stmts.push(quote! {
            let __hs = <#field_ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
            ::pvm_contract_sdk::SolEncode::encode_body_to(&#field_access, &mut buf[__offset..__offset + __hs]);
            __offset += __hs;
        });
    }

    quote! { #(#stmts)* }
}

fn generate_static_decode_body(fields: &Fields, unchecked: bool) -> TokenStream {
    let decoder = |ty: TokenStream| {
        if unchecked {
            quote! {
                let __val = unsafe { <#ty as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(input, offset + __offset) };
            }
        } else {
            quote! {
                let __val = <#ty as ::pvm_contract_sdk::SolDecode>::decode_at(input, offset + __offset)?;
            }
        }
    };
    let res = match fields {
        Fields::Named(named) => {
            let mut pre_stmts: Vec<TokenStream> = vec![quote! { let mut __offset: usize = 0; }];
            let mut field_lets = Vec::new();

            for field in &named.named {
                let name = field.ident.as_ref().unwrap();
                let ty = &field.ty;
                let tmp = quote::format_ident!("__field_{}", name);
                let decoder = decoder(quote! {#ty});
                pre_stmts.push(quote! {
                    let #tmp = {
                        #decoder
                        __offset += <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
                        __val
                    };
                });
                field_lets.push(quote! { #name: #tmp });
            }

            quote! {
                #(#pre_stmts)*
                Self { #(#field_lets),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let mut pre_stmts: Vec<TokenStream> = vec![quote! { let mut __offset: usize = 0; }];
            let mut field_tmps = Vec::new();

            for (i, field) in unnamed.unnamed.iter().enumerate() {
                let ty = &field.ty;
                let tmp = quote::format_ident!("__field_{}", i);
                let decoder = decoder(quote! {#ty});

                pre_stmts.push(quote! {
                    let #tmp = {
                        #decoder
                        __offset += <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
                        __val
                    };
                });
                field_tmps.push(quote! { #tmp });
            }

            quote! {
                #(#pre_stmts)*
                Self(#(#field_tmps),*)
            }
        }
        Fields::Unit => quote! { Self },
    };
    if unchecked {
        quote! {
            #res
        }
    } else {
        quote! {
            Ok({ #res })
        }
    }
}

// -----------------------------------------------------------------------
// Storage codec emission (Solidity-compatible storage layout)
// -----------------------------------------------------------------------

/// Classify a field's storage-layout role.
///
/// Supported:
/// - Packable primitives (right-aligned at sub-word offsets).
/// - Bare `String` / `Vec<u8>` (`SolType::String` / `SolType::DynBytes`) —
///   solc `string` / `bytes` storage layout: header in the struct's slot,
///   body at `keccak256(slot) + i`.
///
/// Unsupported (silently skipped — caller gets a `trait not implemented`
/// error if they try to use the struct as a storage value): nested SolType
/// structs, tuples in struct fields, fixed arrays of non-`u8`,
/// `Vec<T>` for `T != u8`.
#[derive(Debug, Clone, Copy)]
enum StorageFieldKind {
    /// Packs into a parent slot at a sub-word offset.
    Packable,
    /// Solc-style dynamic field (`string` / `bytes`). Occupies one slot
    /// (header); body lives at `keccak256(slot) + i`.
    Dynamic,
    /// Not yet supported as a storage field.
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
        // Custom types (nested SolType structs): we can't determine at macro
        // expansion time whether the referenced type implements
        // `StorageEncode`. A future phase may add an explicit opt-in.
        //
        // `Array<T>` (T != u8), `FixedArray`, `Tuple` in struct fields: deferred.
        SolType::Custom(_) | SolType::Array(_) | SolType::FixedArray(_, _) | SolType::Tuple(_) => {
            StorageFieldKind::Unsupported
        }
    }
}

/// Emit the `StorageEncode` + `StorageDecode` impls for a SolType-derived
/// struct. Supports both static layouts (all fields `Packable`) and
/// dynamic-bodied layouts (fields include `String` / `Bytes` — solc-style
/// header-in-slot + body at `keccak256(slot) + i`). Fields classified as
/// `Unsupported` (nested SolType structs, tuples, fixed arrays of non-`u8`,
/// `Vec<T>` for `T != u8`) fall through to the const-panic stub above.
///
/// Approach:
///   1. Compute a const layout `__STORAGE_LAYOUT = ([(slot, offset); N], total_slots, dynamic_mask)`
///      via a const block that walks fields via `pvm_contract_sdk::layout_step`,
///      using each field type's
///      `<T as StorageEncode>::{PACKED_BYTES, STORAGE_SLOTS}`. `dynamic_mask`
///      records which slot indices are owned by `Dynamic` fields so the
///      encode/decode paths can skip them and defer to the field's own
///      `write_to_storage` / `read_from_storage`.
///   2. Emit `encode_slot` that loops through fields and, for each, conditionally
///      packs (`Packable`) or no-ops (`Dynamic` — handled by `write_to_storage`)
///      when the field belongs to `slot_idx`.
///   3. Emit `from_slots` that decodes each `Packable` field at its precomputed
///      (slot, offset); `Dynamic` fields are reconstituted via the
///      `read_from_storage` override.
///   4. When any field is `Dynamic`, override `write_to_storage` /
///      `read_from_storage` and set `HAS_DYNAMIC_BODY = true`.
fn generate_storage_impls(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    // If any field is not yet storage-compatible (nested custom struct,
    // `Vec<T>` for `T != u8`, fixed array of non-`u8`, tuple), we cannot
    // emit a real `StorageEncode` / `StorageDecode`. Two paths we ruled
    // out:
    //
    // 1. Silent skip (the prior behavior). The user gets a baffling
    //    "trait `StorageEncode` is not implemented for `S`" error
    //    pointing at `Mapping<_, S>` with no hint why.
    // 2. Eager `compile_error!`. Breaks `#[derive(SolType)]` for valid
    //    ABI-only structs (e.g. a function parameter struct that holds
    //    `Vec<U256>` — fine for calldata, irrelevant for storage).
    //
    // What we do instead: emit a stub `StorageEncode` / `StorageDecode`
    // whose `STORAGE_SLOTS` const is `core::panic!(...)`. Const evaluation
    // is deferred to monomorphization — pure-ABI consumers never read
    // `STORAGE_SLOTS`, so they pay nothing. Storage consumers
    // (`Lazy<T>::_SIZE_CHECK`, `Mapping<_, T>::_SIZE_CHECK`, or another
    // derive's storage-layout walker) force evaluation and surface the
    // message verbatim.
    //
    // Caveat: `cargo check` skips MIR-level const evaluation, so the
    // panic only fires during `cargo build` / `cargo test`. This means
    // `trybuild` (which uses `cargo check`) can't pin the error message
    // via a UI fixture. Real users hit `cargo build` and see the message.
    if let Some((field_idx, field_ty, unsupported_ty)) =
        field_info.iter().enumerate().find_map(|(idx, (_, ty))| {
            matches!(classify_storage_field(ty), StorageFieldKind::Unsupported)
                .then(|| (idx, get_field_types(fields)[idx], ty.canonical_name()))
        })
    {
        let field_label = match &field_info[field_idx].0 {
            Some(ident) => format!("field `{ident}`"),
            None => format!("field {field_idx}"),
        };
        let msg = format!(
            "`{name}` cannot be used in on-chain storage: {field_label} has type \
             `{unsupported_ty}` (Rust: `{field_ty_str}`), which is not yet \
             supported as a `StorageEncode` field. Only fixed-size primitives \
             (`uint*`/`int*`/`address`/`bool`/`bytesN`), `string`, and `bytes` \
             (Rust `Bytes`) are supported today. \
             Hint: `#[derive(SolType)]` still emits SolEncode/SolDecode, so \
             the struct remains usable for calldata and event encoding — \
             only `Lazy<{name}>` and `Mapping<_, {name}>` are blocked.",
            name = name,
            field_label = field_label,
            unsupported_ty = unsupported_ty,
            field_ty_str = quote!(#field_ty),
        );

        return Ok(quote! {
            impl ::pvm_contract_sdk::StorageEncode for #name {
                const STORAGE_SLOTS: usize = ::core::panic!(#msg);
                const PACKED_BYTES: usize = 32;

                fn encode_slot(&self, _slot_idx: usize, _buf: &mut [u8; 32]) {
                    unreachable!("storage encode not implemented for {}", stringify!(#name))
                }
            }

            impl ::pvm_contract_sdk::StorageDecode for #name {
                fn from_slots(_slots: &[[u8; 32]]) -> Self {
                    unreachable!("storage decode not implemented for {}", stringify!(#name))
                }
            }
        });
    }

    let field_types: Vec<&Type> = get_field_types(fields);
    let n_fields = field_types.len();

    // ---- const layout walker ----
    //
    // Delegates each step to the shared `layout_step` const fn in
    // `pvm-storage` so the SolType-derive layout stays byte-for-byte
    // aligned with the contract-field chain and the `#[storage]` sub-struct
    // chain. A previous inline copy of this algorithm here was equivalent
    // for all current types but free to drift on future ones; the shared
    // const fn eliminates that risk.
    let walker_steps: Vec<TokenStream> = field_types
        .iter()
        .enumerate()
        .map(|(idx, ty)| {
            quote! {
                {
                    step = ::pvm_contract_sdk::layout_step(
                        step,
                        <#ty as ::pvm_contract_sdk::StorageEncode>::PACKED_BYTES,
                        <#ty as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS as u64,
                    );
                    placements[#idx] = (step.slot as usize, step.offset as usize);
                }
            }
        })
        .collect();

    // Per-field classifications for code generation.
    let kinds: Vec<StorageFieldKind> = field_info
        .iter()
        .map(|(_, ty)| classify_storage_field(ty))
        .collect();
    let has_dynamic = kinds.iter().any(|k| matches!(k, StorageFieldKind::Dynamic));

    // Bitmask of "this field is Dynamic", indexed by field position.
    // Used by the const layout block below to derive the per-slot dynamic
    // mask after the walker has assigned slot indices.
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
            // After the final step, `step.next_slot` is the last slot any
            // field touched; total = next_slot + 1. For an empty struct no
            // step ran, so total is 0.
            let total = if #n_fields == 0 { 0 } else { step.next_slot as usize + 1 };

            // Build a bitmask of slot indices owned by `Dynamic` fields.
            // Each `Dynamic` field has `PACKED_BYTES = 32` and
            // `STORAGE_SLOTS = 1`, so it occupies exactly one slot — flip
            // that bit. The `write_to_storage` override below uses this mask
            // to skip those slots in the encode_slot loop (the field's own
            // `write_to_storage` will handle the header + body in one call,
            // so writing the header twice would defeat the
            // "clear stale body chunks on long→short transitions" logic in
            // `dynamic_bytes_set`).
            let dynamic_field_flags: [bool; #n_fields] = [#(#dynamic_field_flags),*];
            let mut dynamic_mask: u64 = 0;
            let mut __mi: usize = 0;
            while __mi < #n_fields {
                if dynamic_field_flags[__mi] {
                    let s = placements[__mi].0;
                    // Hard `assert!` (not `debug_assert!`) so the error fires
                    // identically in dev and release: in release the
                    // `1u64 << s` shift below would otherwise become a less
                    // legible "left shift overflowed" const-eval panic.
                    assert!(
                        s < 64,
                        "`#[derive(SolType)]`: dynamic field at slot >= 64 \
                         exceeds the u64 `dynamic_mask` bitmask. Reduce the \
                         struct's static-field count or remove the dynamic field.",
                    );
                    dynamic_mask |= 1u64 << s;
                }
                __mi += 1;
            }
            (placements, total, dynamic_mask)
        };
    };

    // ---- encode_slot body ----
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
                // Dynamic fields don't participate in the parent's encode_slot
                // loop. The parent's overridden `write_to_storage` skips
                // dynamic slots (via `dynamic_mask`) and dispatches to each
                // dynamic field's own `write_to_storage` for header + body.
                // Panic if a caller bypasses that path and asks for the
                // dynamic slot directly — matches the trait default on
                // standalone `String` / `Bytes`, which also panics.
                {
                    let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                    if s == slot_idx {
                        unreachable!(
                            "encode_slot called on a dynamic-body slot of `{}`; \
                             callers must dispatch through write_to_storage",
                            stringify!(#name),
                        );
                    }
                }
            },
            StorageFieldKind::Unsupported => unreachable!("rejected above"),
        };
        encode_arms.push(arm);
    }

    // ---- from_slots body ----
    //
    // Static structs decode their slot buffer here. Dynamic-body structs emit
    // a panicking stub: `from_slots` is required on the trait (so static impls
    // can't forget it), but a dynamic-body type genuinely cannot reconstruct
    // from a slot buffer alone — reads must go through `read_from_storage`
    // because the body lives outside the slot buffer.
    let from_slots_impl = if has_dynamic {
        quote! {
            fn from_slots(_slots: &[[u8; 32]]) -> Self {
                unreachable!(
                    "from_slots called on dynamic-body struct `{}`; \
                     callers must dispatch through read_from_storage",
                    stringify!(#name),
                )
            }
        }
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

        let body = match fields {
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
        };

        quote! {
            fn from_slots(slots: &[[u8; 32]]) -> Self {
                #body
            }
        }
    };

    // ---- HAS_DYNAMIC_BODY ----
    let has_dynamic_body_expr = if has_dynamic {
        quote! { true }
    } else {
        quote! { false }
    };

    // Compile-time guard for the `dynamic_mask: u64` bitmask in
    // `__STORAGE_LAYOUT`: with `STORAGE_SLOTS > 64`, the runtime loops in
    // `write_to_storage` / `clear_storage` / `read_from_storage` shift
    // `1u64 << __i` past the bit width of `u64`, which is UB in release
    // builds and panics in debug. Force a clean compile error instead. Only
    // emitted when the struct contains at least one Dynamic field (the only
    // path that uses the mask) — defined here so it's shared between the
    // write/clear and read overrides below.
    //
    // Uses `const { ... }` block expression (not a `const _: () = ...` item)
    // because the block form sees `Self` from the enclosing impl while const
    // items do not.
    let slot_count_assert = quote! {
        let _: () = const {
            assert!(
                <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS <= 64,
                concat!(
                    "`#[derive(SolType)]` on `",
                    stringify!(#name),
                    "`: structs with more than 64 storage slots cannot contain dynamic ",
                    "fields (String / Bytes / nested dynamic structs). The internal ",
                    "`dynamic_mask: u64` bitmask runs out of bits. Either reduce the ",
                    "static-field count or remove the dynamic field.",
                ),
            );
        };
    };

    // ---- write_to_storage / read_from_storage overrides (only if any field
    //      is Dynamic). Default impls are used otherwise.
    let dynamic_impls = if has_dynamic {
        let mut write_calls: Vec<TokenStream> = Vec::new();
        let mut clear_calls: Vec<TokenStream> = Vec::new();

        for (i, (field_name, _)) in field_info.iter().enumerate() {
            let field_ty = field_types[i];
            let field_access = field_access_tokens(fields, i, field_name);
            if let StorageFieldKind::Dynamic = kinds[i] {
                // Header+body write via the dynamic field's own write_to_storage.
                write_calls.push(quote! {
                    {
                        let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                        let mut sub_key = *base_key;
                        for _ in 0..s { __pvm_inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageEncode>::write_to_storage(
                            &#field_access, host, &sub_key,
                        );
                    }
                });
                // Clear via the dynamic field's clear_storage (zeroes header +
                // deletes body chunks).
                clear_calls.push(quote! {
                    {
                        let (s, _) = Self::__STORAGE_LAYOUT.0[#i];
                        let mut sub_key = *base_key;
                        for _ in 0..s { __pvm_inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageEncode>::clear_storage(
                            host, &sub_key, 1,
                        );
                    }
                });
            }
        }

        quote! {
            fn write_to_storage(
                &self,
                host: &::pvm_contract_sdk::Host,
                base_key: &[u8; 32],
            ) {
                #slot_count_assert
                #[inline]
                fn __pvm_inc_be_32(slot: &mut [u8; 32]) {
                    for byte in slot.iter_mut().rev() {
                        let (next, carry) = byte.overflowing_add(1);
                        *byte = next;
                        if !carry { return; }
                    }
                }
                use ::pvm_contract_sdk::{HostApi, StorageFlags};

                // Write each *static* slot's bytes via encode_slot. Slots
                // owned by `Dynamic` fields are skipped — the field's
                // own `write_to_storage` (called below) writes both the
                // header and the body in one operation, and uses the
                // pre-write slot state to decide whether to clear stale
                // body chunks. Pre-writing the header here would defeat
                // that detection (the read would see the new header).
                let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                let mut __k = *base_key;
                for __i in 0..<Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS {
                    if dynamic_mask & (1u64 << __i) == 0 {
                        let mut __buf = [0u8; 32];
                        <Self as ::pvm_contract_sdk::StorageEncode>::encode_slot(self, __i, &mut __buf);
                        host.set_storage_or_clear(StorageFlags::empty(), &__k, &__buf);
                    }
                    __pvm_inc_be_32(&mut __k);
                }

                // Now: write the header+body for each Dynamic field at
                // its derived offset. This is the only path that touches
                // the dynamic field's header slot.
                #(#write_calls)*
            }

            fn clear_storage(
                host: &::pvm_contract_sdk::Host,
                base_key: &[u8; 32],
                _slots: usize,
            ) {
                #slot_count_assert
                #[inline]
                fn __pvm_inc_be_32(slot: &mut [u8; 32]) {
                    for byte in slot.iter_mut().rev() {
                        let (next, carry) = byte.overflowing_add(1);
                        *byte = next;
                        if !carry { return; }
                    }
                }
                use ::pvm_contract_sdk::{HostApi, StorageFlags};

                // Static slots: write zero (auto-deletes via set_storage_or_clear).
                // Dynamic field slots: route through the field's own
                // clear_storage which also deletes body chunks.
                let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                let mut __k = *base_key;
                for __i in 0..<Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS {
                    if dynamic_mask & (1u64 << __i) == 0 {
                        host.set_storage_or_clear(StorageFlags::empty(), &__k, &[0u8; 32]);
                    }
                    __pvm_inc_be_32(&mut __k);
                }
                #(#clear_calls)*
            }
        }
    } else {
        quote! {}
    };

    let read_from_storage_impl = if has_dynamic {
        let mut read_field_lets: Vec<TokenStream> = Vec::new();
        let mut read_construct_fields: Vec<TokenStream> = Vec::new();

        for (i, _) in field_info.iter().enumerate() {
            let field_ty = field_types[i];
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
                        for _ in 0..s { __pvm_inc_be_32(&mut sub_key); }
                        <#field_ty as ::pvm_contract_sdk::StorageDecode>::read_from_storage::<MAX_INLINE_SLOTS>(
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

        quote! {
            fn read_from_storage<const MAX_INLINE_SLOTS: usize>(
                host: &::pvm_contract_sdk::Host,
                base_key: &[u8; 32],
            ) -> Self {
                #slot_count_assert
                #[inline]
                fn __pvm_inc_be_32(slot: &mut [u8; 32]) {
                    for byte in slot.iter_mut().rev() {
                        let (next, carry) = byte.overflowing_add(1);
                        *byte = next;
                        if !carry { return; }
                    }
                }
                use ::pvm_contract_sdk::{HostApi, StorageFlags};

                // Read static-owned slots into a buffer; dynamic-owned slots
                // are skipped here (the field's own `read_from_storage`
                // below SLOADs its header). Mirrors the write side, which
                // skips dynamic slots in the parent's encode_slot loop via
                // the same `dynamic_mask`.
                let dynamic_mask = Self::__STORAGE_LAYOUT.2;
                let mut __slots = [[0u8; 32]; MAX_INLINE_SLOTS];
                let mut __k = *base_key;
                let __n = <Self as ::pvm_contract_sdk::StorageEncode>::STORAGE_SLOTS;
                debug_assert!(__n <= MAX_INLINE_SLOTS);
                for __i in 0..__n {
                    if dynamic_mask & (1u64 << __i) == 0 {
                        host.get_storage_or_zero(StorageFlags::empty(), &__k, &mut __slots[__i]);
                    }
                    __pvm_inc_be_32(&mut __k);
                }

                #(#read_field_lets)*

                #read_construct
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #name {
            #layout_const
        }

        impl ::pvm_contract_sdk::StorageEncode for #name {
            const STORAGE_SLOTS: usize = Self::__STORAGE_LAYOUT.1;
            const PACKED_BYTES: usize = 32;
            const HAS_DYNAMIC_BODY: bool = #has_dynamic_body_expr;

            fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
                *buf = [0u8; 32];
                #(#encode_arms)*
            }

            #dynamic_impls
        }

        impl ::pvm_contract_sdk::StorageDecode for #name {
            #from_slots_impl
            #read_from_storage_impl
        }
    })
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

// -----------------------------------------------------------------------
// Dynamic struct helpers
// -----------------------------------------------------------------------

/// Compute the total head size expression for a dynamic struct.
pub(crate) fn build_dynamic_head_size_expr(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);
    build_dynamic_head_sum_expr(field_info, &field_types)
}

/// Compute the head offset expression for field at position `idx` in a dynamic struct.
fn build_dynamic_field_offset_expr(
    field_info: &[(Option<syn::Ident>, SolType)],
    field_types: &[&Type],
    idx: usize,
) -> TokenStream {
    build_dynamic_head_sum_expr(&field_info[..idx], &field_types[..idx])
}

/// Build a sum expression of dynamic-head contributions for a field slice.
///
/// For known non-custom fields we constant-fold to a literal. When custom types are present,
/// we use trait metadata (`IS_DYNAMIC` and `HEAD_SIZE`) to avoid guessing dynamic/static shape.
fn build_dynamic_head_sum_expr(
    field_info: &[(Option<syn::Ident>, SolType)],
    field_types: &[&Type],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());
    if !has_custom {
        let total: usize = field_info
            .iter()
            .map(|(_, t)| {
                if t.is_dynamic() == Some(true) {
                    32
                } else {
                    t.head_size()
                        .expect("build_dynamic_head_sum_expr called on unresolved custom type")
                }
            })
            .sum();
        return quote! { #total };
    }

    let parts: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((_, t), ty)| match t.is_dynamic() {
            Some(true) => quote! { 32usize },
            Some(false) => {
                let size = t.head_size().unwrap();
                quote! { #size }
            }
            None => quote! {
                <#ty as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE
            },
        })
        .collect();

    quote! { (0 #(+ #parts)*) }
}

pub(crate) fn generate_dynamic_encode_len(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let field_types = get_field_types(fields);
    let tail_lens: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .enumerate()
        .filter_map(|(i, ((field_name, sol_type), field_ty))| {
            let field_access = match fields {
                Fields::Named(_) => {
                    let name = field_name.as_ref().unwrap();
                    quote! { self.#name }
                }
                Fields::Unnamed(_) => {
                    let idx = syn::Index::from(i);
                    quote! { self.#idx }
                }
                Fields::Unit => return None,
            };

            match sol_type.is_dynamic() {
                Some(true) => Some(quote! {
                    ::pvm_contract_sdk::SolEncode::encode_body_len(&#field_access)
                }),
                Some(false) => None,
                None => Some(quote! {
                    if <#field_ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                        ::pvm_contract_sdk::SolEncode::encode_body_len(&#field_access)
                    } else {
                        0usize
                    }
                }),
            }
        })
        .collect();

    quote! {
        #head_size_expr #(+ #tail_lens)*
    }
}

pub(crate) fn generate_dynamic_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let field_types = get_field_types(fields);
    let mut stmts = Vec::new();

    for (i, (field_name, sol_type)) in field_info.iter().enumerate() {
        let field_access = match fields {
            Fields::Named(_) => {
                let name = field_name.as_ref().unwrap();
                quote! { self.#name }
            }
            Fields::Unnamed(_) => {
                let idx = syn::Index::from(i);
                quote! { self.#idx }
            }
            Fields::Unit => continue,
        };

        let head_offset_expr = build_dynamic_field_offset_expr(field_info, &field_types, i);
        let field_ty = field_types[i];

        match sol_type.is_dynamic() {
            Some(true) => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        buf[__ho..__ho + 24].fill(0);
                        buf[__ho + 24..__ho + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                        let __tail_len = ::pvm_contract_sdk::SolEncode::encode_body_len(&#field_access);
                        ::pvm_contract_sdk::SolEncode::encode_body_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                        __tail_offset += __tail_len;
                    }
                });
            }
            Some(false) => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        ::pvm_contract_sdk::SolEncode::encode_body_to(&#field_access, &mut buf[__ho..]);
                    }
                });
            }
            None => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        if <#field_ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                            buf[__ho..__ho + 24].fill(0);
                            buf[__ho + 24..__ho + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                            let __tail_len = ::pvm_contract_sdk::SolEncode::encode_body_len(&#field_access);
                            ::pvm_contract_sdk::SolEncode::encode_body_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                            __tail_offset += __tail_len;
                        } else {
                            ::pvm_contract_sdk::SolEncode::encode_body_to(&#field_access, &mut buf[__ho..]);
                        }
                    }
                });
            }
        }
    }

    quote! {
        let mut __tail_offset: usize = #head_size_expr;
        #(#stmts)*
    }
}

pub fn generate_dynamic_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);
    match fields {
        Fields::Named(named) => {
            let field_decodes: Vec<_> = named
                .named
                .iter()
                .zip(field_info.iter())
                .enumerate()
                .map(|(i, (field, (field_name, sol_type)))| {
                    let name = field_name.as_ref().unwrap();
                    let ty = &field.ty;
                    let head_offset_expr =
                        build_dynamic_field_offset_expr(field_info, &field_types, i);
                    let decode = generate_dynamic_field_decode(ty, sol_type, &head_offset_expr);
                    quote! {
                        #name: #decode
                    }
                })
                .collect();

            quote! {
                Ok(Self { #(#field_decodes),* })
            }
        }
        Fields::Unnamed(unnamed) => {
            let field_decodes: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(field_info.iter())
                .enumerate()
                .map(|(i, (field, (_, sol_type)))| {
                    let ty = &field.ty;
                    let head_offset_expr =
                        build_dynamic_field_offset_expr(field_info, &field_types, i);
                    generate_dynamic_field_decode(ty, sol_type, &head_offset_expr)
                })
                .collect();

            quote! {
                Ok(Self(#(#field_decodes),*))
            }
        }
        Fields::Unit => quote! { Ok(Self) },
    }
}

fn generate_dynamic_field_decode(
    ty: &Type,
    sol_type: &SolType,
    head_offset_expr: &TokenStream,
) -> TokenStream {
    // Offsets read from calldata are attacker-controlled, so every composition
    // routes through checked arithmetic (`read_word_offset` / `checked_sum`) —
    // matching the hand-written decoders in `pvm-contract-types`. A raw `+`
    // here would wrap silently under `overflow-checks = false` and alias reads
    // back into the calldata buffer instead of failing closed.
    match sol_type.is_dynamic() {
        Some(true) => quote! {{
            let __ho = #head_offset_expr;
            let __field_offset = ::pvm_contract_sdk::read_word_offset(
                input,
                ::pvm_contract_sdk::checked_sum([offset, __ho])?,
            )?;
            <#ty as ::pvm_contract_sdk::SolDecode>::decode_tail(
                input,
                ::pvm_contract_sdk::checked_sum([offset, __field_offset])?,
            )?
        }},
        Some(false) => quote! {{
            let __ho = #head_offset_expr;
            <#ty as ::pvm_contract_sdk::SolDecode>::decode_at(
                input,
                ::pvm_contract_sdk::checked_sum([offset, __ho])?,
            )?
        }},
        None => quote! {{
            let __ho = #head_offset_expr;
            if <#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                let __field_offset = ::pvm_contract_sdk::read_word_offset(
                    input,
                    ::pvm_contract_sdk::checked_sum([offset, __ho])?,
                )?;
                <#ty as ::pvm_contract_sdk::SolDecode>::decode_tail(
                    input,
                    ::pvm_contract_sdk::checked_sum([offset, __field_offset])?,
                )?
            } else {
                <#ty as ::pvm_contract_sdk::SolDecode>::decode_at(
                    input,
                    ::pvm_contract_sdk::checked_sum([offset, __ho])?,
                )?
            }
        }},
    }
}

pub(crate) fn extract_field_info(
    fields: &Fields,
) -> syn::Result<Vec<(Option<syn::Ident>, SolType)>> {
    let mut result = Vec::new();

    match fields {
        Fields::Named(named) => {
            for field in &named.named {
                let sol_type = type_to_sol_type(&field.ty)?;
                result.push((field.ident.clone(), sol_type));
            }
        }
        Fields::Unnamed(unnamed) => {
            for field in &unnamed.unnamed {
                let sol_type = type_to_sol_type(&field.ty)?;
                result.push((None, sol_type));
            }
        }
        Fields::Unit => {}
    }

    Ok(result)
}

fn type_to_sol_type(ty: &Type) -> syn::Result<SolType> {
    SolType::from_rust_type(ty).ok_or_else(|| {
        syn::Error::new_spanned(
            ty,
            "Unsupported type for SolType derive. Supported types: \
                 U256, u128, u64, u32, u16, u8, I256, i128, i64, i32, i16, i8, \
                 bool, Address, [u8; N] (bytesN), String, Bytes (bytes), \
                 Vec<T> (T[]), [T; N] (T[N]), tuples (T1, T2, …). \
                 For custom structs, derive SolType on them first."
                .to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::SolType;

    fn normalize_tokens(ts: TokenStream) -> String {
        ts.to_string().split_whitespace().collect::<String>()
    }

    #[test]
    fn custom_type_field_total_size_uses_trait_expression() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("x").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("count").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
        ];
        let expr = build_total_size_expr(&field_info);
        let expected = quote! { 0 + 32usize + <Count as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_sol_name_expr_uses_concatcp_for_custom_types() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![(
            Some(syn::parse_str::<syn::Ident>("count").unwrap()),
            SolType::Custom("Count".to_string()),
        )];
        let expr = build_sol_name_expr(&field_info);
        let expected = quote! {
            ::pvm_contract_sdk::const_format::concatcp!(
                "(",
                <Count as ::pvm_contract_sdk::SolEncode>::SOL_NAME,
                ")"
            )
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_sol_name_expr_uses_literal_for_known_types() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("x").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("y").unwrap()),
                SolType::Uint(64),
            ),
        ];
        let expr = build_sol_name_expr(&field_info);
        let expected = quote! { "(uint64,uint64)" };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn dynamic_head_size_uses_trait_dynamic_for_custom_types() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                a: Count,
                b: u64,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("a").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("b").unwrap()),
                SolType::Uint(64),
            ),
        ];

        let expr = build_dynamic_head_size_expr(fields, &field_info);
        let expected = quote! {
            (0 +
                <Count as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE
                + 32usize)
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn dynamic_field_offset_uses_trait_dynamic_for_custom_prefix() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                a: Count,
                b: u64,
                c: bool,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_types = get_field_types(fields);
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("a").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("b").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("c").unwrap()),
                SolType::Bool,
            ),
        ];

        let expr = build_dynamic_field_offset_expr(&field_info, &field_types, 2);
        let expected = quote! {
            (0 +
                <Count as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE
                + 32usize)
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_is_dynamic_expr_uses_trait_for_custom_types() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                point: Point,
                value: u64,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("point").unwrap()),
                SolType::Custom("Point".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("value").unwrap()),
                SolType::Uint(64),
            ),
        ];

        let expr = build_is_dynamic_expr(fields, &field_info);
        let expected = quote! {
            false || <Point as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC || false
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn expand_sol_type_does_not_force_true_is_dynamic_for_static_custom_struct() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct Line {
                a: Point,
                b: Point,
            }
        };

        let expanded = normalize_tokens(expand_sol_type(input).unwrap());
        let expected_is_dynamic = normalize_tokens(quote! {
            const IS_DYNAMIC: bool = false
                || <Point as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC
                || <Point as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC;
        });
        assert!(expanded.contains(&expected_is_dynamic));
    }

    #[test]
    fn expand_sol_type_keeps_known_dynamic_fields_in_is_dynamic_expr() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct NamedPoint {
                point: Point,
                name: alloc::string::String,
            }
        };

        let expanded = normalize_tokens(expand_sol_type(input).unwrap());
        let expected_is_dynamic = normalize_tokens(quote! {
            const IS_DYNAMIC: bool = false
                || <Point as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC
                || true;
        });
        assert!(expanded.contains(&expected_is_dynamic));
    }
}
