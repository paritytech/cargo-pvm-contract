#[cfg(feature = "alloc")]
extern crate alloc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use pvm_contract_types::{SolDecode, SolEncode};
use ruint::aliases::U256;

// ============================================================================
// Primitives: u8
// ============================================================================

fn bench_u8_encode_pvm(c: &mut Criterion) {
    let value = black_box(42u8);
    c.bench_function("u8_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = [0u8; 32];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

fn bench_u8_decode_pvm(c: &mut Criterion) {
    let value = 42u8;
    let mut buf = [0u8; 32];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("u8_decode_pvm", |b| {
        b.iter(|| {
            let decoded = u8::decode(&buf);
            black_box(decoded)
        });
    });
}

fn bench_u8_encode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = black_box(42u8);
    c.bench_function("u8_encode_alloy", |b| {
        b.iter(|| {
            let encoded = alloy_core::primitives::U256::from(value).abi_encode();
            black_box(encoded)
        });
    });
}

fn bench_u8_decode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = 42u8;
    let encoded = black_box(alloy_core::primitives::U256::from(value).abi_encode());
    c.bench_function("u8_decode_alloy", |b| {
        b.iter(|| {
            let decoded = alloy_core::primitives::U256::abi_decode(&encoded).unwrap();
            let decoded = decoded.to::<u8>();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Primitives: u32
// ============================================================================

fn bench_u32_encode_pvm(c: &mut Criterion) {
    let value = black_box(0x12345678u32);
    c.bench_function("u32_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = [0u8; 32];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

fn bench_u32_decode_pvm(c: &mut Criterion) {
    let value = 0x12345678u32;
    let mut buf = [0u8; 32];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("u32_decode_pvm", |b| {
        b.iter(|| {
            let decoded = u32::decode(&buf);
            black_box(decoded)
        });
    });
}

fn bench_u32_encode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = black_box(0x12345678u32);
    c.bench_function("u32_encode_alloy", |b| {
        b.iter(|| {
            let encoded = alloy_core::primitives::U256::from(value).abi_encode();
            black_box(encoded)
        });
    });
}

fn bench_u32_decode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = 0x12345678u32;
    let encoded = black_box(alloy_core::primitives::U256::from(value).abi_encode());
    c.bench_function("u32_decode_alloy", |b| {
        b.iter(|| {
            let decoded = alloy_core::primitives::U256::abi_decode(&encoded).unwrap();
            let decoded = decoded.to::<u32>();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Primitives: u128
// ============================================================================

fn bench_u128_encode_pvm(c: &mut Criterion) {
    let value = black_box(0x0123456789abcdef0123456789abcdefu128);
    c.bench_function("u128_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = [0u8; 32];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

fn bench_u128_decode_pvm(c: &mut Criterion) {
    let value = 0x0123456789abcdef0123456789abcdefu128;
    let mut buf = [0u8; 32];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("u128_decode_pvm", |b| {
        b.iter(|| {
            let decoded = u128::decode(&buf);
            black_box(decoded)
        });
    });
}

fn bench_u128_encode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = black_box(0x0123456789abcdef0123456789abcdefu128);
    c.bench_function("u128_encode_alloy", |b| {
        b.iter(|| {
            let encoded = alloy_core::primitives::U256::from(value).abi_encode();
            black_box(encoded)
        });
    });
}

fn bench_u128_decode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = 0x0123456789abcdef0123456789abcdefu128;
    let encoded = black_box(alloy_core::primitives::U256::from(value).abi_encode());
    c.bench_function("u128_decode_alloy", |b| {
        b.iter(|| {
            let decoded = alloy_core::primitives::U256::abi_decode(&encoded).unwrap();
            let decoded = decoded.to::<u128>();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Primitives: U256
// ============================================================================

fn bench_u256_encode_pvm(c: &mut Criterion) {
    let value = black_box(U256::from_limbs([
        0x0123456789abcdef,
        0xfedcba9876543210,
        0x1111111111111111,
        0x2222222222222222,
    ]));
    c.bench_function("u256_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = [0u8; 32];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

fn bench_u256_decode_pvm(c: &mut Criterion) {
    let value = U256::from_limbs([
        0x0123456789abcdef,
        0xfedcba9876543210,
        0x1111111111111111,
        0x2222222222222222,
    ]);
    let mut buf = [0u8; 32];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("u256_decode_pvm", |b| {
        b.iter(|| {
            let decoded = U256::decode(&buf);
            black_box(decoded)
        });
    });
}

fn bench_u256_encode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = black_box(alloy_core::primitives::U256::from(0x0123456789abcdefu64));
    c.bench_function("u256_encode_alloy", |b| {
        b.iter(|| {
            let encoded = value.abi_encode();
            black_box(encoded)
        });
    });
}

fn bench_u256_decode_alloy(c: &mut Criterion) {
    use alloy_core::sol_types::SolValue;
    let value = alloy_core::primitives::U256::from(0x0123456789abcdefu64);
    let encoded = black_box(value.abi_encode());
    c.bench_function("u256_decode_alloy", |b| {
        b.iter(|| {
            let decoded = alloy_core::primitives::U256::abi_decode(&encoded).unwrap();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Address: [u8; 20]
// ============================================================================

fn bench_address_encode_pvm(c: &mut Criterion) {
    let value = black_box([
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc,
    ]);
    c.bench_function("address_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = [0u8; 32];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

fn bench_address_decode_pvm(c: &mut Criterion) {
    let value = [
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc,
    ];
    let mut buf = [0u8; 32];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("address_decode_pvm", |b| {
        b.iter(|| {
            let decoded = <[u8; 20]>::decode(&buf);
            black_box(decoded)
        });
    });
}

fn bench_address_encode_alloy(c: &mut Criterion) {
    use alloy_core::primitives::Address;
    use alloy_core::sol_types::SolValue;
    let value = black_box(Address::from([
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc,
    ]));
    c.bench_function("address_encode_alloy", |b| {
        b.iter(|| {
            let encoded = value.abi_encode();
            black_box(encoded)
        });
    });
}

fn bench_address_decode_alloy(c: &mut Criterion) {
    use alloy_core::primitives::Address;
    use alloy_core::sol_types::SolValue;
    let value = Address::from([
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc,
    ]);
    let encoded = black_box(value.abi_encode());
    c.bench_function("address_decode_alloy", |b| {
        b.iter(|| {
            let decoded = Address::abi_decode(&encoded).unwrap();
            black_box(decoded)
        });
    });
}

// ============================================================================
// String (requires alloc feature)
// ============================================================================

#[cfg(feature = "alloc")]
fn bench_string_encode_pvm(c: &mut Criterion) {
    use alloc::string::String;
    let value = black_box(String::from("Hello, Solidity!"));
    c.bench_function("string_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = vec![0u8; value.encode_len()];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_string_decode_pvm(c: &mut Criterion) {
    use alloc::string::String;
    let value = String::from("Hello, Solidity!");
    let mut buf = vec![0u8; value.encode_len()];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("string_decode_pvm", |b| {
        b.iter(|| {
            let decoded = String::decode(&buf);
            black_box(decoded)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_string_encode_alloy(c: &mut Criterion) {
    use alloc::string::String;
    use alloy_core::sol_types::SolValue;
    let value = black_box(String::from("Hello, Solidity!"));
    c.bench_function("string_encode_alloy", |b| {
        b.iter(|| {
            let encoded = value.abi_encode();
            black_box(encoded)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_string_decode_alloy(c: &mut Criterion) {
    use alloc::string::String;
    use alloy_core::sol_types::SolValue;
    let value = String::from("Hello, Solidity!");
    let encoded = black_box(value.abi_encode());
    c.bench_function("string_decode_alloy", |b| {
        b.iter(|| {
            let decoded = String::abi_decode(&encoded).unwrap();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Vec<U256> (requires alloc feature)
// ============================================================================

#[cfg(feature = "alloc")]
fn bench_vec_u256_encode_pvm(c: &mut Criterion) {
    use alloc::vec;
    let value = black_box(vec![
        U256::from(1u64),
        U256::from(2u64),
        U256::from(3u64),
        U256::from(4u64),
    ]);
    c.bench_function("vec_u256_encode_pvm", |b| {
        b.iter(|| {
            let mut buf = vec![0u8; value.encode_len()];
            value.encode_to(&mut buf);
            black_box(buf)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_vec_u256_decode_pvm(c: &mut Criterion) {
    use alloc::vec;
    let value = vec![
        U256::from(1u64),
        U256::from(2u64),
        U256::from(3u64),
        U256::from(4u64),
    ];
    let mut buf = vec![0u8; value.encode_len()];
    value.encode_to(&mut buf);
    let buf = black_box(buf);
    c.bench_function("vec_u256_decode_pvm", |b| {
        b.iter(|| {
            let decoded = Vec::<U256>::decode(&buf);
            black_box(decoded)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_vec_u256_encode_alloy(c: &mut Criterion) {
    use alloc::vec;
    use alloy_core::sol_types::SolValue;
    let value = black_box(vec![
        alloy_core::primitives::U256::from(1u64),
        alloy_core::primitives::U256::from(2u64),
        alloy_core::primitives::U256::from(3u64),
        alloy_core::primitives::U256::from(4u64),
    ]);
    c.bench_function("vec_u256_encode_alloy", |b| {
        b.iter(|| {
            let encoded = value.abi_encode();
            black_box(encoded)
        });
    });
}

#[cfg(feature = "alloc")]
fn bench_vec_u256_decode_alloy(c: &mut Criterion) {
    use alloc::vec;
    use alloy_core::sol_types::SolValue;
    let value = vec![
        alloy_core::primitives::U256::from(1u64),
        alloy_core::primitives::U256::from(2u64),
        alloy_core::primitives::U256::from(3u64),
        alloy_core::primitives::U256::from(4u64),
    ];
    let encoded = black_box(value.abi_encode());
    c.bench_function("vec_u256_decode_alloy", |b| {
        b.iter(|| {
            let decoded = Vec::<alloy_core::primitives::U256>::abi_decode(&encoded).unwrap();
            black_box(decoded)
        });
    });
}

// ============================================================================
// Criterion Groups
// ============================================================================

criterion_group!(
    primitives_u8,
    bench_u8_encode_pvm,
    bench_u8_decode_pvm,
    bench_u8_encode_alloy,
    bench_u8_decode_alloy
);

criterion_group!(
    primitives_u32,
    bench_u32_encode_pvm,
    bench_u32_decode_pvm,
    bench_u32_encode_alloy,
    bench_u32_decode_alloy
);

criterion_group!(
    primitives_u128,
    bench_u128_encode_pvm,
    bench_u128_decode_pvm,
    bench_u128_encode_alloy,
    bench_u128_decode_alloy
);

criterion_group!(
    primitives_u256,
    bench_u256_encode_pvm,
    bench_u256_decode_pvm,
    bench_u256_encode_alloy,
    bench_u256_decode_alloy
);

criterion_group!(
    address,
    bench_address_encode_pvm,
    bench_address_decode_pvm,
    bench_address_encode_alloy,
    bench_address_decode_alloy
);

#[cfg(feature = "alloc")]
criterion_group!(
    string,
    bench_string_encode_pvm,
    bench_string_decode_pvm,
    bench_string_encode_alloy,
    bench_string_decode_alloy
);

#[cfg(feature = "alloc")]
criterion_group!(
    vec_u256,
    bench_vec_u256_encode_pvm,
    bench_vec_u256_decode_pvm,
    bench_vec_u256_encode_alloy,
    bench_vec_u256_decode_alloy
);

#[cfg(feature = "alloc")]
criterion_main!(
    primitives_u8,
    primitives_u32,
    primitives_u128,
    primitives_u256,
    address,
    string,
    vec_u256
);

#[cfg(not(feature = "alloc"))]
criterion_main!(
    primitives_u8,
    primitives_u32,
    primitives_u128,
    primitives_u256,
    address
);
