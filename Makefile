.PHONY: transit transit-wasm transit-native proto-example

transit: transit-wasm transit-native

transit-wasm:
	@echo "=== Building transit for wasm32"
	cargo build -p transit-core --release --features client --target wasm32-unknown-unknown

transit-native:
	@echo "=== Building transit for native"
	cargo build -p transit-core --release --features server,client,uniffi

example:
	@echo "=== Building example"
	cd examples/idp-proto && cargo build --release --features client
