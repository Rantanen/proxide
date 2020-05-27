# Proxide
### HTTP2/gRPC Debugging Proxy

[![crates.io](https://img.shields.io/crates/v/proxide.svg)](https://crates.io/crates/proxide)

![Demo](images/proxide.gif)

## Installation

```
cargo install proxide
```

## Usage

Run the proxide UI listening on port `1234`, bridging connections to
`localhost:8888` and using `my.proto`, `dependent.proto` and `third.proto` gRPC
descriptions to decode the traffic.

> ```
> proxide monitor -l 1234 -t localhost:8888 --grpc my.proto dependent.proto third.proto
> ```

Bridge the local port `8888` to `remote.server:8888` while capturing the
network traffic to file `capture.bin` for later analysis.

> ```
> proxide capture capture.bin -l 8888 -t remote.server:8888
> ```

View the previously captured file uing `service.proto` to decode the gRPC
traffic.

> ```
> proxide view capture.bin --grpc service.proto
> ```

## Status

**Proxide is currently under development**

The basic decoding works, but there are still few "production quality" features
missing.

- [x] Proxy arbitrary HTTP/2 traffic.
- [x] Decode gRPC communication.
  - [x] Support multiple proto-files and/or proto-file with `import` statements.
- [ ] Better TUI tooling.
  - [ ] Search/highlight support.
  - [ ] Clipboard integration.
    - [x] Well we got request/response exporting at least!
  - [x] Follow communication streams.
  - [ ] Switch between different encoders manually (Raw, Headers, gRPC).
- [ ] Better support for corrupted/incomplete message display.
- [x] Import/Export session.
- [ ] Support streaming JSON/Protobuf/etc. output for graphical UI integration.
- [ ] Support TLS
- [ ] Support HTTP/1.x upgrades
- [ ] Support for acquiring stacktraces from local requests with thread-id
  headers.
- [ ] HTTP/1.x support
