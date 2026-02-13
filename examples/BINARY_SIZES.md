# Binary Sizes

PolkaVM binary sizes for `examples/example-mytoken` variants.

## How to regenerate

Run the helper script:

```bash
./scripts/regenerate-example-binary-sizes.sh
```

This script builds release artifacts and rewrites this file.

## Release Profile

| Binary | Flavor | Size (bytes) | Size |
|--------|--------|-------------:|-----:|
| example-mytoken-macro-no-alloc | macro-no-alloc | 3,794 | 3.7 KB |
| example-mytoken-dsl-no-alloc | dsl-no-alloc | 4,104 | 4.0 KB |
| example-mytoken-macro-bump-alloc | macro-bump-alloc | 4,703 | 4.6 KB |
| example-mytoken-alloy-alloc | alloy-alloc | 5,832 | 5.7 KB |
| example-mytoken-macro-pico-alloc | macro-pico-alloc | 16,405 | 16.0 KB |
