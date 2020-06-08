# Proxide
### HTTP2/gRPC Debugging Proxy

[![crates.io](https://img.shields.io/crates/v/proxide.svg)](https://crates.io/crates/proxide)
[![CI](https://github.com/Rantanen/proxide/workflows/CI/badge.svg)](https://github.com/Rantanen/proxide/actions?query=workflow%3ACI+branch%3Amaster)

![Demo](images/proxide.gif)

## Installation

See the [releases](https://github.com/Rantanen/proxide/releases) page for
binary releases for Windows and Linux.

Proxide can also be installed directly from `crates.io` with:

```
cargo install proxide
```

## Getting started

*In addition to the instructions below, Proxide provides (hopefully)
comprehensive help through the `--help` command line argument.*

There are few different ways to use Proxide to monitor client traffic. The big
choices are in how Proxide intercepts the client traffic and whether the user
wants to monitor the traffic in real time or record it for later analysis.

The examples below are written considering the case where the user wants to
monitor traffic in real time using the Proxide UI. Replacing `monitor` with
`capture -f output_file` allows the user to capture the traffic directly into a
file for later analysis.

### Direct connection to Proxide

The most straight forward way to run Proxide is to use it to have the clients
connect directly to Proxide and have Proxide redirect these connections to a
remote server.

> ```
> proxide monitor -l 5555 -t example.com:8080
> ```

Proxide will automatically rewrite outgoing `authority` information with that
of the target server, making the redirection transparent on the HTTP/2 protocol
level. However if the client includes the server address in the actual payload
or if the server includes its address in the responses, these are not altered,
making the presence of the proxy visible to the parties and possibly breaking
the communication.

### CONNECT proxy

In case where it's important that the client is able to use the real server
address, but still route the traffic through Proxide, Proxide can be used as a
CONNECT proxy. This happens automatically if a target server is not specified.

> ```
> proxide monitor -l 5555
> ```

When acting as a CONNECT proxy, the client needs to be configured to route its
traffic through the proxy. The exact way in which this configuration is made
depends on the client. Many clients support `http_proxy` environment variable
for this.

> ```
> http_proxy=http://localhost:5555 ./grpc_application
> ```

### Viewing captured traffic

Previously captured files (and exported sessions) can be viewed with `proxide
view`.

> ```
> proxide view capture.bin
> ```

### Decoding gRPC

When Proxide is used to analyze gRPC traffic, it helps to be able to decode the
messages. Proxie requires the service and message definitions to be able to do
this.  These definitions can be given to Proxide with the `--grpc` option. The
option works with both the `monitor` and `view` commands.

> ```
> proxide view capture.bin --grpc /project/src/*.proto
> ```

### Decoding TLS

*Note that trusting CA certificates may compromise the system security. Please
take your time to understand the full implications of this.*

Intercepting and decoding TLS traffic requires Proxide to perform a
man-in-the-middle attack on the client. This is something TLS is designed to
protect against so the user needs to perform some setup to allow this.

First, Proxide needs a *CA certificate*. This certificate is used for
generating *server certificates* served to the client. The server certificates
are generated on the fly as Proxide intercepts client connections, but all of
them are signed by the CA certificate. The easiest way to generate such CA
certificate is through the `config ca` command.

> ```
> proxide config ca --create
> ```

This command creates a `proxide_ca.crt` and `proxide_ca.key` pair. If these
files exist, Proxide will automatically use them when monitoring or capturing
traffic.

The second obstacle is ensuring the client won't reject the server certificates
Proxide creates. This can be done by having the client ignore certificate
issues or specifying the Proxide CA certificate as a trusted root CA
certificate for the client.

The exact details on how this is done depend on the client and platform. For
example on Windows many clients use the Windows certificate store. Proxide is
able to add the root CA to this store with the `--trust` option. If a
certificate is aded to the platform certificate store, it is recommended to
remove it from that store when the debugging session is over. This can be done
with the `--revoke` option.


> ```
> proxide config ca --trust
> ```

(This can be combined with the `--create` flag when creating the certificate in
the first place.)

## Status

**Proxide is currently under development**

The basic decoding works, but there are still few "production quality" features
missing.

- [ ] Better TUI tooling.
  - [ ] Search/highlight support.
  - [ ] Switch between different encoders manually (Raw, Headers, gRPC).
- [ ] Support streaming JSON/Protobuf/etc. output for graphical UI integration.
  - [x] There's _some_ support for this used in integration tests.
- [ ] Support for acquiring stacktraces from local requests with thread-id
  headers.
- [ ] Support HTTP/1.x upgrades
- [ ] HTTP/1.x support
