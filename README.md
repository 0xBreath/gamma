# Development
1. Make sure you have rust installed

2. Install tools and build everything
```rust
just build-local
```

# Testing
Runs LiteSVM test `programs/gamma/tests/test_market.rs` which simulates the lifecycle of a market with two outcomes, A and B.
```rust
just test
```

# TODO
- [ ] Resolve market at `resolve_at` by checking which outcome has the most reserves