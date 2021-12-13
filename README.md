# `measure-wasm-dedupe-wins`

This is a tool to measure the amount of available wins from adding various kinds
of deduplication to Wasmtime's caching and/or in-memory representations for a
given corpus of Wasm binaries.

## Usage

```
$ cargo build --release
$ cargo run --release -- path/to/corpus/of/Wasm/binaries
Total size:                   9706508 bytes
--------------------------------------------------------------------------------
Duplicated data segments:       79230 bytes (0.82%)
Duplicated elem segments:         368 bytes (0.00%)
Duplicated code bodies:        100858 bytes (1.04%)
Duplicated custom sections:   3404222 bytes (35.07%)
--------------------------------------------------------------------------------
Total duplicated data:        3584678 bytes (36.93%)
```
