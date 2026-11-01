# transit

The first binary communication protocol with almost no overhead[^1].

## What I wanted to do when I started this project

1. As few bits as possible over the wire
2. API is useless unless the client and server have the same route definitions.
3. Use `uniffi` to generate client bindings for other languages (specifically, Go)
4. Transport-agnostic! Should work over raw TCP/TLS streams, WebSockets, WebTransport, or literally anything else capable of long-lived bidirectional streaming.
5. Works on the web
6. Incredibly simple to declare schemas, errors, and routes

## Documentation

[the docs](https://codeberg.org/tendulkar/transit/src/branch/main/docs/docs.md)

---

[^1]: Transit has only 20 bytes of overhead for requests and 12 bytes for responses. Everything else is _your data_. Pretty crazy.
