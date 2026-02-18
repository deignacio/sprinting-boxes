# sprinting-boxes

The core Rust application for Sprinting Boxes, providing a CLI and a web server to host the interactive dashboard.

## Overview

Sprinting Boxes is a platform for analyzing sprint training and team sports. This crate handles:
- **Run Context Management**: Automated workspace creation and metadata persistence.
- **API Server**: An Axum-based REST API serving run data and static assets.
- **Embedded Dashboard**: A premium React-based UI for managing analysis sessions.

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

## Output Data

The pipeline generates a `features.csv` file in the output directory with the following columns:

- `frame_index`: Frame number.
- `left_count`, `right_count`: Normalized player counts in end zones.
- `field_count`: Normalized player count in the field.
- `pre_point_score`: Heuristic score for potential points.
- `is_cliff`: Boolean indicating a "cliff" event (sudden drop in players).
- `com_x`, `com_y`: Normalized Center of Mass of players (0.0-1.0).
- `distribution_std_dev`: Normalized standard deviation of player positions relative to CoM.
- `com_delta_x`, `com_delta_y`: Frame-to-frame change in CoM.
- `std_dev_delta`: Frame-to-frame change in StdDev.
