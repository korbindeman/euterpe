target := "thumbv7em-none-eabihf"
binary := "preamp_daisy"

default: format clippy

check-format:
    cargo fmt --all -- --check

format:
    cargo fmt --all

# Host crates on the host, firmware and no_std crates on the Daisy target.
clippy: clippy-host clippy-firmware

firmware_crates := "-p preamp_daisy -p daisy_runtime"
no_std_crates := "-p euterpe_core -p euterpe_demo -p dsp -p preamp"

clippy-host:
    cargo clippy --workspace --exclude preamp_daisy --exclude daisy_runtime --all-targets -- -D warnings

clippy-firmware:
    cargo clippy {{firmware_crates}} {{no_std_crates}} --target {{target}} -- -D warnings

# Every crate type-checks for the target it runs on.
check:
    cargo check --workspace --exclude preamp_daisy --exclude daisy_runtime --all-targets
    cargo check {{firmware_crates}} {{no_std_crates}} --target {{target}}

update:
    cargo update

build:
    cargo build --release -p {{binary}} --target {{target}}

flash: build
    rust-objcopy -O binary target/{{target}}/release/{{binary}} target/{{binary}}.bin
    dfu-suffix --vid 0x0483 --pid 0xdf11 --add target/{{binary}}.bin
    dfu-util -a 0 -s 0x08000000:leave -D target/{{binary}}.bin -d ,0483:df11 || true

dev:
    cargo run -p preamp_desktop

# Euterpe CLI, e.g. `just euterpe render demo.drive --gen sine:220 --out out.wav`
euterpe *args:
    cargo run -q --release -p euterpe_cli -- {{args}}

# Test the Euterpe crates only (no hardware or desktop dependencies)
test-euterpe:
    cargo test -p euterpe_core -p euterpe_offline -p euterpe_cli -p euterpe_demo
