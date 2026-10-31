# transit

The first[^1] binary communication protocol with almost no overhead[^2].

## Considerations

1. As few bits as possible over the wire
2. Types are contracts! APIs are useless unless the client and server have the same route definitions.
3. Use `uniffi` to generate client bindings for other languages (Go has been tested)
4. Transport-agnostic! Can work over raw TCP/TLS streams, WebSockets, WebTransport, or literally anything else capable of long-lived bidirectional streaming.[^3]
5. Works on the web! Just bundle your WebAssembly
6. Incredibly simple to declare schemas, errors, and routes

---

[^1]: Debatable, I didn't look too far behind the popular RPCs.

[^2]: Transit has only 20 bytes of overhead for requests and 12 bytes for responses. Everything else is _your data_. Pretty crazy.

[^3]: I only implemented TCP/TLS streams and WebSockets
