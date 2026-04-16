# i3status-dumb

My tiny status generator for `swaybar` or `i3bar`.

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
- active keyboard layout
- local clock, updated every second

## How It Works

The program runs three async watchers and prints a new line whenever any of them changes state.

- `src/volume.rs`
  Uses `pactl subscribe` for push events, then asks `wpctl get-volume @DEFAULT_AUDIO_SINK@` for current sink volume.
- `src/layout.rs`
  Auto-detects the session. On Sway it connects directly to `SWAYSOCK`, fetches current inputs, subscribes to `input` events, and extracts `xkb_active_layout_name`. Outside Sway it falls back to `setxkbmap -query`, which works for my i3/X11 setup.
- `src/clock.rs`
  Uses a `tokio` interval ticking once per second.
- `src/main.rs`
  Merges watcher updates through `tokio::sync::watch` channels and prints one new line on every change.

## Assumptions

This repo assumes a setup close to mine:

- Rust toolchain
- `pactl`
- `wpctl`
- either Sway with `SWAYSOCK` set, or i3/X11 with `setxkbmap`

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
- `setxkbmap` command available if you want i3/X11 layout support
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

Inside your WM session:

```sh
./target/release/i3status-dumb
```

You should see live-updating lines on stdout.

## Use With `swaybar` or `i3bar`

Example `~/.config/sway/config` or `~/.config/i3/config` block:

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

- Layout backend is auto-detected.
- If `SWAYSOCK` is set, the Sway IPC watcher is used.
- Otherwise the program falls back to `setxkbmap -query` for i3/X11.
- If neither path works, layout stays `??`.
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
- direct Sway IPC when available
- simple X11 fallback for i3
- event-driven volume refresh
- tiny codebase

## Current Limits

- only tested against my own Sway and i3-style setups
- output format hardcoded
- no battery, network, CPU, RAM, weather, or fancy bar protocol

If you want all of that, this repo is probably the wrong starting point.
