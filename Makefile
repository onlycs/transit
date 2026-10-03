.PHONY: transit transit-wasm transit-native proto-example

transit: transit-wasm transit-native
transit-native: transit-native-client-asyncio transit-native-client-tokio transit-native-uniffi transit-native-server
transit-tokio: transit-wasm transit-native-client-tokio transit-native-server
transit-asyncio: transit-native-client-asyncio transit-native-uniffi

transit-wasm:
	@echo "=== Building transit for wasm32"
	cargo build -p transit-core --release --features client,tokio --target wasm32-unknown-unknown

transit-native-client-asyncio:
	@echo "=== Building transit (client, async-io) for native"
	cargo build -p transit-core --release --features client,async-io --target x86_64-unknown-linux-gnu

transit-native-client-tokio:
	@echo "=== Building transit (client, tokio) for native"
	cargo build -p transit-core --release --features client,tokio --target x86_64-unknown-linux-gnu

transit-native-uniffi:
	@echo "=== Building transit (uniffi) for native"
	cargo build -p transit-core --release --features uniffi --target x86_64-unknown-linux-gnu

transit-native-server:
	@echo "=== Building transit (server) for native"
	cargo build -p transit-core --release --features server --target x86_64-unknown-linux-gnu

example:
	@echo "=== Building example"
	cd examples/idp-proto && cargo build --release --features client
