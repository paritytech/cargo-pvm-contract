# Binary Sizes

PolkaVM binary sizes for `examples/example-mytoken` variants.

## How to regenerate

Run the helper script:

```bash
./scripts/regenerate-example-binary-sizes.sh
```

This script regenerates the examples, builds release artifacts, and rewrites this file.

## Release Profile

| Binary | Flavor | Size (bytes) | Size |
|--------|--------|-------------:|-----:|
| example-mytoken-dsl-no-alloc | dsl-no-alloc | 3,763 | 3.7 KB |
| example-mytoken-macro-no-alloc | macro-no-alloc | 3,751 | 3.7 KB |
| example-mytoken-macro-alloc | macro-alloc | 16,205 | 15.8 KB |
| example-mytoken-alloy-alloc | alloy-alloc | 17,243 | 16.8 KB |
