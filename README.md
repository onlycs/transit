# transit

The first[^1] binary commmunication protocol with literally no overhead[^2].

## Considerations

1. As few bits as possible over the wire
2. Types are contracts! APIs are useless unless the client and server have the same route definitions (which is basically mandated)
3. Use `uniffi` to generate client bindings for other languages (Go has been tested)
4. Transport-agnostic! Can work over raw TCP/TLS streams, WebSockets, WebTransport, or literally anything else capable of long-lived bidirectional streaming.[^3]
5. Works on the web! Just bundle your WebAssembly
6. Incredibly simple to declare schemas, errors, and routes (see example)

---

[^1]: Debatable, I didn't look too far beyond RPC

[^2]: Except for one: we must encode the frame length and message IDs. But every byte after that is yours

[^3]: I only implemented TCP/TLS streams and WebTransport
