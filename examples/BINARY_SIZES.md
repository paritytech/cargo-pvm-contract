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
| example-mytoken-macro-no-alloc | macro-no-alloc | 3,696 | 3.6 KB |
| example-mytoken-dsl-no-alloc | dsl-no-alloc | 4,081 | 4.0 KB |
| example-mytoken-macro-bump-alloc | macro-bump-alloc | 4,734 | 4.6 KB |
| example-mytoken-alloy-alloc | alloy-alloc | 5,767 | 5.6 KB |
| example-mytoken-macro-no-sol | macro-no-sol | 16,480 | 16.1 KB |
| example-mytoken-macro-pico-alloc | macro-pico-alloc | 16,480 | 16.1 KB |
