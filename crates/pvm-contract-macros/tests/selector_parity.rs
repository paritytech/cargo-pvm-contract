extern crate alloc;
pub use pvm_contract_sdk::*;

#[test]
#[allow(clippy::too_many_arguments)]
fn selector_parity() {
    mod t {
        use super::*;
        abi_import! {
            #![abi_import(alloc = true)]
            // SPDX-License-Identifier: MIT
            pragma solidity ^0.8.0;

            struct Point {
                uint a;
                uint b;
            }

            interface PointAdder {
                function add(Point a, Point b) external returns (Point);
            }
        }
    }
    mod alloy {
        use alloy_core::sol;

        sol! {
            pragma solidity ^0.8.0;

            struct Point2 {
                uint a;
                uint b;
            }

            contract PointAdderr {
                function add(Point2 a, Point2 b) external returns (Point2);
            }
        }
    }
    let mut input1 = vec![0u8; 5000];

    let mut out = vec![0u8; 5000];
    let mock_host = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock_host.clone()));
    let _ = t::point_adder::PointAdder::from_address(Address([0u8; 20]))
        .add(
            t::Point {
                a: U256::from(1),
                b: U256::from(1),
            },
            t::Point {
                a: U256::from(1),
                b: U256::from(1),
            },
        )
        .call_raw(&mut Context::new(host), &mut input1, &mut out);
    use alloy_core::sol_types::SolCall;

    let alloy = alloy::PointAdderr::addCall {
        a: alloy::Point2 {
            a: U256::from(1),
            b: U256::from(1),
        },
        b: alloy::Point2 {
            a: U256::from(1),
            b: U256::from(1),
        },
    }
    .abi_encode();
    input1.truncate(alloy.len());
    assert_eq!(input1, alloy)
}

/// The enum half of "a user-defined type canonicalizes to its ABI form".
/// `selector_parity` covers the struct half; an enum takes the other branch
/// (`CustomDef::Enum` -> `uint8`), so a regression there — `Color` hashing as
/// anything but `uint8` — moves the selector without touching a struct.
///
/// The `uint8` form is asserted elsewhere against hand-computed constants
/// (`abi_import_no_alloc_static_custom`) and against our own snapshots
/// (`abi_output::enum_abi`); alloy is the only *independent* oracle, which is
/// what this test buys.
#[test]
#[allow(clippy::too_many_arguments)]
fn selector_parity_enum() {
    mod t {
        use super::*;
        abi_import! {
            #![abi_import(alloc = true)]
            // SPDX-License-Identifier: MIT
            pragma solidity ^0.8.0;

            enum Color { Red, Green, Blue }

            interface Picker {
                function pick(Color c) external returns (Color);
            }
        }
    }
    mod alloy {
        use alloy_core::sol;

        sol! {
            pragma solidity ^0.8.0;

            enum Color2 { Red, Green, Blue }

            contract Pickerr {
                function pick(Color2 c) external returns (Color2);
            }
        }
    }
    let mut input = vec![0u8; 256];
    let mut out = vec![0u8; 256];
    let mock_host = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock_host.clone()));
    let _ = t::picker::Picker::from_address(Address([0u8; 20]))
        .pick(t::Color::Blue)
        .call_raw(&mut Context::new(host), &mut input, &mut out);

    use alloy_core::sol_types::SolCall;

    let alloy = alloy::Pickerr::pickCall {
        c: alloy::Color2::Blue,
    }
    .abi_encode();
    input.truncate(alloy.len());
    assert_eq!(input, alloy)
}
