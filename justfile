target := "thumbv7em-none-eabihf"
binary := "preamp_daisy"

default: format clippy

check-format:
    cargo fmt --all -- --check

format:
    cargo fmt --all

# Crates that only build for the Daisy target, and the no_std crates that
# also get checked for it.
firmware_crates := "-p preamp_daisy -p daisy_runtime -p euterpe_daisy -p demo_daisy -p euterpe_bench_qemu"
no_std_crates := "-p euterpe_core -p euterpe_demo -p euterpe_bench -p dsp -p preamp"
host_excludes := "--exclude preamp_daisy --exclude daisy_runtime --exclude euterpe_daisy --exclude demo_daisy --exclude euterpe_bench_qemu"

# Host crates on the host, firmware and no_std crates on the Daisy target.
clippy: clippy-host clippy-firmware

clippy-host:
    cargo clippy --workspace {{host_excludes}} --all-targets -- -D warnings

clippy-firmware:
    cargo clippy {{firmware_crates}} {{no_std_crates}} --target {{target}} -- -D warnings

# Every crate type-checks for the target it runs on.
check:
    cargo check --workspace {{host_excludes}} --all-targets
    cargo check {{firmware_crates}} {{no_std_crates}} --target {{target}}

# Tests for every host crate.
test:
    cargo test --workspace {{host_excludes}}

update:
    cargo update

build:
    cargo build --release -p {{binary}} --target {{target}}

# Flash a firmware crate over DFU, e.g. `just flash demo_daisy`.
flash bin=binary:
    cargo build --release -p {{bin}} --target {{target}}
    rust-objcopy -O binary target/{{target}}/release/{{bin}} target/{{bin}}.bin
    dfu-suffix --vid 0x0483 --pid 0xdf11 --add target/{{bin}}.bin
    dfu-util -a 0 -s 0x08000000:leave -D target/{{bin}}.bin -d ,0483:df11 || true

# Instruction counts per block on an emulated Cortex-M7, checked against the
# baseline. Needs qemu-system-arm, cc + glib headers, and llvm-tools.
bench *args:
    cargo run -q --release -p euterpe_cli -- bench --baseline benches/qemu-baseline.txt {{args}}

# Accept the current instruction counts as the new baseline.
bench-baseline:
    cargo run -q --release -p euterpe_cli -- bench --write-baseline benches/qemu-baseline.txt

dev:
    cargo run -p preamp_desktop

# Euterpe CLI, e.g. `just euterpe render demo.drive --gen sine:220 --out out.wav`
euterpe *args:
    cargo run -q --release -p euterpe_cli -- {{args}}

# Test the Euterpe crates only (no hardware or desktop dependencies)
test-euterpe:
    cargo test -p euterpe_core -p euterpe_offline -p euterpe_cli -p euterpe_demo
