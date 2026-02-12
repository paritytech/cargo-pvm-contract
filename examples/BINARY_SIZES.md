# Binary Sizes

PolkaVM binary sizes for all scaffolded example contracts.

## How to regenerate

Build every example and measure the `.polkavm` output:

```bash
for dir in examples/*/; do
  (cd "$dir" && env -u CARGO -u RUSTUP_TOOLCHAIN cargo +nightly build --release 2>/dev/null)
done
```

The `.polkavm` files are written to each project's `target/` directory.
To regenerate the examples themselves, run `./scripts/regenerate-examples.sh`.

## Release Profile

### blank

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-blank-no-alloc | new | no-alloc | 302 | 0.3 KB |
| new-blank-alloc | new | alloc | 12,061 | 11.8 KB |

### fibonacci

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-from-sol-fibonacci-no-alloc | new-from-sol | no-alloc | 362 | 0.4 KB |
| example-fibonacci-no-alloc | example | no-alloc | 472 | 0.5 KB |
| new-from-sol-fibonacci-alloc | new-from-sol | alloc | 12,175 | 11.9 KB |
| example-fibonacci-alloc | example | alloc | 12,312 | 12.0 KB |

### mytoken

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-from-sol-mytoken-no-alloc | new-from-sol | no-alloc | 714 | 0.7 KB |
| example-mytoken-no-alloc | example | no-alloc | 3,751 | 3.7 KB |
| new-from-sol-mytoken-alloc | new-from-sol | alloc | 12,484 | 12.2 KB |
| example-mytoken-alloc | example | alloc | 16,205 | 15.8 KB |

## Debug Profile

### blank

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-blank-no-alloc | new | no-alloc | 26,116 | 25.5 KB |
| new-blank-alloc | new | alloc | 59,732 | 58.3 KB |

### fibonacci

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-from-sol-fibonacci-no-alloc | new-from-sol | no-alloc | 28,475 | 27.8 KB |
| example-fibonacci-no-alloc | example | no-alloc | 35,802 | 35.0 KB |
| new-from-sol-fibonacci-alloc | new-from-sol | alloc | 62,853 | 61.4 KB |
| example-fibonacci-alloc | example | alloc | 70,343 | 68.7 KB |

### mytoken

| Project | Type | Memory Model | Size (bytes) | Size |
|---------|------|--------------|-------------:|-----:|
| new-from-sol-mytoken-no-alloc | new-from-sol | no-alloc | 38,642 | 37.7 KB |
| example-mytoken-no-alloc | example | no-alloc | 50,062 | 48.9 KB |
| new-from-sol-mytoken-alloc | new-from-sol | alloc | 73,003 | 71.3 KB |
| example-mytoken-alloc | example | alloc | 86,164 | 84.1 KB |
