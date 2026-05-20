# recalculate

A CLI tool for recalculating pp and user stats for refx.

## What it does

1. **Score recalculation** — fetches all best scores from the database, recomputes pp using [refx-pp](https://github.com/refx-online/refx-pp-rs), and updates the `scores` table.
2. **User stat recalculation** — recomputes weighted pp and accuracy for every user, updates the `stats` table, and refreshes Redis leaderboards.

Supports both legacy and lazer scores, including custom clock rates and relax/autopilot modes.

## Supported game modes

| ID | Mode |
|----|------|
| 0 | osu! |
| 1 | Taiko |
| 2 | Catch |
| 3 | Mania |
| 4 | osu! Relax |
| 5 | Taiko Relax |
| 6 | Catch Relax |
| 8 | osu! Autopilot |
| 12 | osu! Cheat |
| 16 | osu! CheatCheat |
| 20 | Touch Device |

## Setup

Copy `.env.example` to `.env` and fill in the values:

```env
DATABASE_URL=mysql://bancho:password@localhost:3306/bancho
REDIS_URL=redis://localhost:6379
BEATMAPS_PATH=/path/to/beatmaps
BEATMAPS_SERVICE_URL=
```

## Usage

```
recalc [OPTIONS]

Options:
  -d, --debug          Enable debug logging
      --no-scores      Skip score recalculation
      --no-stats       Skip user stat recalculation
  -m, --mode <MODE>    Comma-separated list of mode IDs to process
                       [default: 0,1,2,3,4,5,6,8,12,16,20]
  -h, --help           Print help
```

### Examples

Recalculate everything:
```sh
recalc
```

Only recalculate scores for osu! standard and taiko:
```sh
recalc --no-stats -m 0,1
```

Only recalculate user stats:
```sh
recalc --no-scores
```

## Docker

Build and run with Docker using the provided Makefile:

```sh
make build
make run
# Pass CLI args via ARGS:
make run ARGS="--no-stats -m 0,1"
```

The container mounts the `meat-my-beat-i_data` volume to `/srv/root/.data` for beatmap storage and uses `--network=host` to reach the database and Redis.

## Building from source

Requires Rust (edition 2024).

```sh
cargo build --release
# Binary at: target/release/recalc
```
