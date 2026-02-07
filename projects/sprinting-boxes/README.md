# sprinting-boxes

The core Rust application for Sprinting Boxes, providing a CLI and a web server to host the dashboard.

## Development

Run the application:
```bash
cargo run -- --host 127.0.0.1 --port 12206
```

## Build

Build the application:
```bash
cargo build --release
```

## Code Quality

Check style:
```bash
cargo fmt -- --check
```

Lint code:
```bash
cargo clippy -- -D warnings
```

Format code:
```bash
cargo fmt
```
