# i3status-dumb

My tiny status generator for `swaybar`.

I wrote this for my own setup and I am mostly throwing the source code out into the wild in case it is useful to someone else. It is not meant to be a general-purpose status bar or a polished replacement for anything.

Prints one plain text line:

```text
42% us 2026-04-15 09:49:57 PM
```

The goal is simple:

- no giant config system
- no JSON protocol layer
- no polling spam for layout
- fast updates when volume or keyboard layout changes

Right now it shows:

- volume from PipeWire or PulseAudio tools
- active keyboard layout from Sway IPC
- local clock, updated every second

## How It Works

The program runs three async watchers and prints a new line whenever any of them changes state.

- `src/volume.rs`
  Uses `pactl subscribe` for push events, then asks `wpctl get-volume @DEFAULT_AUDIO_SINK@` for current sink volume.
- `src/sway.rs`
  Connects directly to `SWAYSOCK`, fetches current inputs, subscribes to `input` events, then extracts `xkb_active_layout_name`.
- `src/clock.rs`
  Uses a `tokio` interval ticking once per second.
- `src/main.rs`
  Merges watcher updates through `tokio::sync::watch` channels and prints one new line on every change.

## Assumptions

This repo assumes a setup close to mine:

- Rust toolchain
- `pactl`
- `wpctl`
- Sway with `SWAYSOCK` set

On Arch I would install:

```sh
sudo pacman -S rust pipewire-pulse wireplumber
```

On Debian or Ubuntu, probably:

```sh
sudo apt install cargo pulseaudio-utils wireplumber pipewire-bin
```

If your distro splits packages differently, the important part is:

- `pactl` command available
- `wpctl` command available
- active PipeWire or PulseAudio-compatible audio session

## Build

```sh
cargo build --release
```

Binary path:

```text
target/release/i3status-dumb
```

## Run

Inside Sway session:

```sh
./target/release/i3status-dumb
```

You should see live-updating lines on stdout.

## Use With `swaybar`

Example `~/.config/sway/config` block:

```conf
bar {
    status_command /path/to/i3status-dumb/target/release/i3status-dumb
}
```

If you install system-wide:

```sh
sudo install -m755 target/release/i3status-dumb /usr/local/bin/i3status-dumb
```

Then:

```conf
bar {
    status_command i3status-dumb
}
```

## Behavior Notes

- If `SWAYSOCK` missing, layout stays `??`.
- If audio tools cannot reach session daemon, volume stays `??%`.
- Layout mapping currently special-cases:
  - `English (US)` -> `us`
  - `Russian` -> `ru`
  - `Ukrainian` -> `ua`
- Any other layout falls back to first 3 lowercase chars of layout name.

## Why This Exists

I did not want a pile of modules, a config format, or shell scripts spawning commands every second.

This stays intentionally small:

- one binary
- one line of output
- direct Sway IPC for layout
- event-driven volume refresh
- tiny codebase

## Current Limits

- built for Sway sessions, not generic X11/i3 IPC
- output format hardcoded
- no battery, network, CPU, RAM, weather, or fancy bar protocol

If you want all of that, this repo is probably the wrong starting point.
