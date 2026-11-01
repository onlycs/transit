# So what does transit do, actually?

It's RPC. You send a thing, you get a thing back. You write a route like this, once, in a crate both sides see:

```rust
route! {
    GetUser(GetUserRequest) -> Result<User, GetUserError>;
}
```

the server registers a handler for it

```rust
async fn get_user(req: GetUserRequest, state: Arc<MyState>) -> Result<User, GetUserError> { ... }

// in main():
Router::new(MyState::default())
    .route::<GetUser, _>(get_user)
```

and then the client does `client.route::<GetUser>(req).await` and gets a `Result` back.

No protobuf, no codegen. Your Rust is the schema; done.

## ok but why not use (insert thing here)

- **Route IDs are hashes of the types.** `xxh3(name + request type + response type)`. If you change a struct on the server and forget to update a client, you get "unknown route" instead of cursed garbage you spend half a year debugging. This is honestly the main reason Transit exists.
- **Doesn't care about your runtime.** The runtime trait is two functions, write your own if you want.
- **Doesn't care about your transport, either.** TCP, TLS on top of whatever (mTLS too if you're into that), websockets. Also a trait.
- **Other languages just work.** `route!` spits out uniffi bindings (Kotlin, Swift, etc) and typescript stuff for the browser.

## stuff it does that you don't have to think about

- A bunch of requests share one connection
- Client reconnects if the connection died
- You can leave a connection open as long as you want
- Requests time out (60s default, change it)
- If your handler panics the client gets an `InternalError`.[^1] Every route's error type has `InternalError` in it automatically. You're welcome.

## What does transit _not_ do?

- streaming or server push (yet, but I won't implement client-to-server streams, that's kinda useless).
- compression (yet).
- schema files for non-Rust people.
- connection striping/inverse multiplexing/splitting a request across TcpStreams to make it go faster (if I do this, it will not be anytime soon).
- Also, client and server both need to agree on `unstable`, more on this later.

# How do I protocol?

## 1. Make a new rust library:

```sh
cargo new --lib src-proto
```

## 2. Add a ton of dependencies and features (I might remove the need for some of this later):

```toml
[package]
name = "my-proto"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[features]
default = []
uniffi = [
    "dep:uniffi",
    "dep:transit-client",
    "dep:transit-ffi",
    "dep:transit-tls",
    "transit-core/uniffi",
]
nightly = [
    "transit-core/nightly",
    "snafu/unstable-provider-api",
    "transit-client?/nightly",
    "transit-tls?/nightly",
    "transit-ffi?/nightly",
    "transit-wasm/nightly",
]

[dependencies]
bitcode = "0.6.9"
serde = { version = "1.0", features = ["derive"] }
snafu = { version = "0.9.2" }
transit-ffi = { git = "https://codeberg.org/tendulkar/transit", optional = true }
transit-client = { git = "https://codeberg.org/tendulkar/transit", optional = true }
transit-core = { git = "https://codeberg.org/tendulkar/transit" }
transit-tls = { git = "https://codeberg.org/tendulkar/transit", optional = true }
type-signature = "0.1.3"
xxhash-rust = { version = "0.8.18", features = ["const_xxh3"] }

[target.'cfg(not(target_family = "wasm"))'.dependencies]
uniffi = { version = "=0.31.0", optional = true, features = ["cli"] }

[target.'cfg(target_family = "wasm")'.dependencies]
transit-wasm = { git = "https://codeberg.org/tendulkar/transit" }
serde-wasm-bindgen = "0.6.5"
tsify = { version = "0.5.8", default-features = false, features = ["js"] }
js-sys = "0.3"
wasm-bindgen = "0.2.128"
wasm-bindgen-futures = "0.4.78"
```

## 3. Do a little bit of this in your `lib.rs`

```rust
#[cfg(all(target_family = "wasm", feature = "uniffi"))]
compile_error!("`wasm32` is not compatible with `uniffi`");

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("my_proto");
```

## 4. It's schemas time

```rust
#[constvar]
pub const MY_CONSTANT: u32 = 0; // exports as functions to wasm and ffi

#[oneof]
pub enum MyEnum {
    Unit,
    TupleLike(String),
    StructLike { my_field: String },
}

#[record]
pub struct MyStruct {
    pub my_field: String,
}

error! {
    SomethingHappened("Uh oh");
    SomethingHappenedWithFmt("Oh, look, formatting: {field:x}") { field: u64 };

    // note: SomethingHappened is just a struct, you can import it from other
    //       files and use it as part of a different error
    // note: MyError can be any of the errors you described, OR an internal
    //       error.
    MyError = SomethingHappened | SomethingHappenedWithFmt;
}

route! {
    MyRoute(MyEnum) -> Result<MyStruct, MyError>;
}
```

# How do I server?

```rust
async fn get_user(req: GetUserRequest, state: Arc<MyState>) -> Result<User, GetUserError> { ... }

// in main():
let router = Router::new(MyState::default())
    .route::<GetUser, _>(get_user)
    .build();

transit_server::serve::<MyRuntime, MyTransport, _>(my_transport_options, ServeOptions::default(), router).await?;

// concrete example:
transit_server::serve::<RtTokio, TransitTcp, _>(socket_addr, ServeOptions::default(), router).await?;
```

Notes:

- `Router::build()` puts it into an `Arc` so it can be cloned. You can then serve transit using multiple transports at the same time.
- Set up `tracing` and `tracing_subscriber`
- Handlers need to be Send/Sync.
- You can opt to not have a state by using the `_stateless` functions
- Errors are a little overengineered

# How do I error?

## 0. Bailing

You can do a quick

```rust
return Err(SomethingHappened.into());
```

## 1. Converting between protocol errors

In the example above, `MyError` generates `MyErrorCast`. If you have another error type that is defined as:

```rust
MyOtherError = SomethingHappened | SomethingHappenedWithFmt | OptionallyOtherStuff;
```

then, you can:

```rust
fn my_func(...) -> Result<T, MyOtherError> {
    let res: Result<T, MyError> = ...;
    let res: T = res.context(MyErrorCast::<MyOtherError>)?;
}
```

This uses `snafu`'s already-existing `ResultExt`.

## 2. Converting from a non-protocol error to a protocol error

Use `TransitErrorContext!()`:

```rust
fn my_func(...) -> Result<T, MyError> {
    let res: Result<T, ExternalError> = ...;
    let res: T = res.context(TransitErrorContext!(SomethingHappened))?;
}
```

#### Notes

- `TransitErrorContext` logs a `warn!()` with a lot more information about your error.
- If your error isn't a `std::error::Error`, use `via(display)`:

```rust
TransitErrorContext!(via(display), SomethingHappened)
```

## 3. Converting from a non-protocol error to an internal error

### 3.1. Using `InternalSnafu`

```rust
fn my_func(...) -> Result<T, MyError> {
    let res: Result<T, ExternalError> = ...;
    let res: T = res.context(InternalSnafu)?;
}
```

#### Notes

- the error's `fmt::Display` becomes the message, potentially leaking sensitive information. Don't do this unless you're lazier than me.

### 3.2. Using `InternalErrorContext!()`

```rust
fn my_func(...) -> Result<T, MyError> {
    let res: Result<T, ExternalError> = ...;
    let res: T = res.context(InternalErrorContext!("Uh oh, but formatting: {}", 4))?;
}
```

#### Notes

- Similar to `TransitErrorContext`, this also logs a `warn!()`
- Similar to `TransitErrorContext`, use `via(display)` for when the external error type doesn't implement `std::error::Error`

### 3.3. But I don't have a `Result`

Use `InternalError!()`, the macro:

```rust
InternalError!(ctx(none), "My message with formatting {}", 6); // no source error. this is kinda ugly, see 3.4.
InternalError!(ctx(display my_source_error), "My message, more formatting {}", 7); // logs fmt::Display of the source with a warn!()
InternalError!(ctx(source_error), "Even more formatting {}", 9); // logs a bunch of information about std::error::Error's
```

### 3.4. Working with `snafu::whatever!()`

For the lazy among you, `snafu::whatever!()` does convert to an `InternalError` (yay!)

```rust
fn my_func(...) -> Result<T, MyError> {
    if x < 5 {
        snafu::whatever!("Oh no something bad!!");
    }

    snafu::ensure_whatever!(x > 3, "Something very bad!!");
}
```

## 4. Using protocol errors elsewhere

All protocol errors implement `std::error::Error` so as to not be a pain. You should probably be fine just using them wherever.

# How do I client?

Pick your language. Rust is the nice one, everything else goes through bindings.

## Rust

Add the dependencies

```toml
transit-client = { git = "https://codeberg.org/tendulkar/transit" }
transit-tcp = { git = "https://codeberg.org/tendulkar/transit", features = ["client"] }
transit-tokio = { git = "https://codeberg.org/tendulkar/transit" }
transit-tls = { git = "https://codeberg.org/tendulkar/transit", features = ["client"] }
```

Then:

```rust
use transit_client::{TransitClient, TransitOptions};
use transit_tcp::TransitTcp;
use transit_tokio::RtTokio;

let client = TransitClient::<RtTokio, TransitTcp>::connect(
    TransitOptions::new("127.0.0.1:8080".to_string()),
).await?;

let user = client.route::<GetUser>(GetUserRequest { id: 4 }).await??;
```

Yes, that's two `?`s. The outer one is transit stuff (couldn't connect, timed out, connection died). The inner one is your route's error, the one you wrote in `error!`.

TLS wraps another transport:

```rust
use transit_tls::{TransitTls, client::{RootCa, TlsOptions}};

let client = TransitClient::<RtTokio, TransitTls<TransitTcp>>::connect(
    TlsOptions {
        inner: "example.com:8080".to_string(),
        server_name: "example.com".to_string(),
        ca: RootCa::Internal, // system certs. RootCa::Bytes(pem) for your own CA
        client: None,         // Some(ClientAuth { .. }) for mTLS
    },
    ClientOptions::default(), // change timeout, etc. here
).await?;
```

### Things to know

- Make one client and share it (`Arc` it). `route` takes `&self` and requests multiplex over one connection. Don't make a client per request, that's silly.
- Timeout: `TransitOptions { timeout: Duration::from_secs(10), ..TransitOptions::new(addr) }`
- Reconnecting is lazy. If the connection died, the _next_ request reconnects. Requests that were in flight when it died fail with "request closed" and don't get retried. Retry them yourself if you care.
- Client and server need the same `unstable` setting, or every request fails. (again, more on this later)

## Uniffi

You already did the annoying part in the crazy long Cargo.toml in your protocol crate.

```sh
cd src-proto
cargo build --release --features uniffi  # Build with uniffi
uniffi-bindgen-go target/release/libmy_proto.so -o ../path/to/bindings -c ../path/to/uniffi.toml # Generate your bindings (use a different uniffi-bindgen for other languages). Ensure compatibility with uniffi 0.31.0, or things break.

# Now build your application. This is language dependent, and there are too many languages for me to cover. Go look at the uniffi documentation.
```

### Things to know

- Everything is `suspend` / `async`. You don't need a Rust runtime on your side, the FFI client brings its own executor thread.
- FFI is TCP or TCP+TLS only. No websockets. You want other stuff, make it yourself, it's easy.
- Logs go to stdout. On Android that's nowhere. Sorry. You want it somewhere else? Bring your own `tracing_subscriber`.

## Browser (wasm)

Browsers only speak websockets, so the server has to serve with `TransitWs`:

```rust
transit_server::serve::<RtTokio, TransitWs<TransitTcp>, _>(
    WsListenOptions { inner: addr, path: "rpc".into(), config: WebSocketConfig { ... } },
    ServeOptions::default(),
    router,
).await?;
```

For `wss://` in this example, put a reverse proxy in front. `TransitTls<TransitWs>` compiles, but it does TLS _inside_ the websocket, and no browser will talk to that. Actually, I _guess_ you could make that work if you tried hard enough. If you don't have a reverse proxy, use `TransitWs<TransitTls<TransitTcp>>`

Build without the `uniffi` feature (it'll yell at you otherwise):

```sh
wasm-pack build --target web
```

Then:

```ts
import init, { Transit, route_get_user } from "./pkg/my_proto.js";

await init();
const transit = await Transit.connect("wss://example.com/rpc");

const res = await route_get_user(transit, { id: 4 });
if ("Ok" in res) {
  console.log(res.Ok);
} else {
  console.error(res.Err); // { Protocol: ... } or { Transit: ... }
}
```

### Things to know

- Route errors don't throw. The promise resolves to an `Ok`/`Err` object. It only rejects if your request doesn't match the type. `Transit.connect` does throw though.
- Types are in `pkg/my_proto.d.ts`. Go look, they're right there.

# What is the unstable thing I keep talking about

The team at `rustc` are working on language goals which include compile-time reflection in the const evaluator. This is useful for us because we can hash types we don't own (which is a pain otherwise). That being said, the entire thing is currently in nightly. The reason why I didn't put it in the `nightly` feature is because it requires the client and the server to both use/not use unstable. The existing `nightly` feature just controls `error_generic_member_access`, which no one cares about.

And when I say unstable I mean it's _really_ unstable. Just look at this

```rust
#![cfg_attr(
    feature = "unstable",
    feature(
        type_info,
        const_cmp,
        const_trait_impl,
        const_index,
        rustc_attrs,
        f16,
        f128
    ),
    allow(internal_features)
)]
```

Now _that's_ a lot of feature flags. _And_ `rustc_attrs` is in there too. Whenever the Rust language developers (finally) finish this new fantastic const system,[^2] we're switching over.

[^1]: This assumes you're using `panic = "unwind"`.

[^2]: const/consteval is my single favorite feature of Rust, and its beyond cool that we're getting something as good as, if not better than Zig's `comptime` in the coming years.
