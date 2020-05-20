# Proxide
### HTTP2/gRPC Debugging Proxy

![Demo](images/proxide.gif)

## Usage

Run proxide listening on port `1234`, bridging connections to port `8888` and
using `my.proto` gRPC description to decode the traffic.

```
cargo run -- -l 1234 -t 8888 -p my.proto
```

## Status

**Proxide is currently under development**

The basic decoding works, but there are still few "production quality" features
missing.

- [x] Proxy arbitrary HTTP/2 traffic.
- [x] Decode gRPC communication.
  - [ ] Support multiple proto-files and/or proto-file with `import` statements.
- [ ] Better TUI tooling.
  - [ ] Search/highlight support.
  - [ ] Clipboard integration.
  - [ ] Follow communication streams.
  - [ ] Switch between different encoders manually (Raw, Headers, gRPC).
- [ ] Better support for corrupted/incomplete message display.
- [ ] Import/Export session.
- [ ] Support streaming JSON/Protobuf/etc. output for graphical UI integration.
- [ ] Support TLS
- [ ] Support HTTP/1 upgrades

