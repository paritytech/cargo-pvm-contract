# Development Notes

## Documentation Maintenance

When modifying code generation in `crates/pvm-contract-macros/src/codegen/`:
- Update the corresponding doc examples in `crates/pvm-contract-macros/src/lib.rs`
- Regenerate docs with `cargo doc --no-deps -p pvm-contract-macros`
- Verify examples still accurately reflect generated code

Key doc sections to keep in sync:
- `#[contract]` macro: dispatch logic examples (alloc and no_alloc modes)
- `#[derive(SolType)]` macro: generated impl block example
