# i3status-dumb

A tiny status generator for `swaybar`.

The name is now a lie.

This is no longer for i3. It is for `sway`. It used to shell out to random desktop tools like a tiny goblin. That goblin has been fired.

Now it is written properly in Rust:

* `swayipc-async` for keyboard layout events
* `libpulse-binding` for volume and mute changes
* no `pactl`
* no `swaymsg`
* no `setxkbmap`
* no shell commands at runtime

It still prints one plain text line:

```text
42% us 2026-04-24 09:49:57 PM
```

## What It Is

A deliberately small status command for people who want:

* one binary
* one line of output
* no JSON bar protocol
* no config language
* no shell scripts glued together with spite

It watches three things:

* default sink volume and mute state
* active Sway keyboard layout
* local clock

When something changes, it prints a fresh line.

## Philosophy

Not literally suckless. Same idea:

* small codebase
* hardcoded behavior on purpose
* minimal runtime dependencies
* no knobs unless they earn their keep
* talk to real APIs, not wrapper commands

This is not a framework. It is not extensible. It is not trying to be helpful.

It does one job and stops.

## How It Works

* `src/layout.rs`
  Talks to Sway over IPC and listens for input events.
* `src/volume.rs`
  Connects to PulseAudio-compatible servers and listens for changes.
* `src/clock.rs`
  Ticks once per second.
* `src/main.rs`
  Merges state and prints the line.

## Scope

Supported:

* `sway`
* PulseAudio or PipeWire (PulseAudio compatibility)
* plain text output

Not supported:

* `i3`
* X11
* shell fallbacks
* “just one more metric” requests

If you want a general-purpose status system, this is the wrong tool.

## X11 / i3 Support

Gone for now.

If you really want it, use v0.2.0:
[https://github.com/Gur0v/i3status-dumb/releases/tag/v0.2.0](https://github.com/Gur0v/i3status-dumb/releases/tag/v0.2.0)

If I ever end up using i3 again, which I probably will not, I will add X11 support back.

## Build

You need Rust and PulseAudio client libraries.

Arch:

```sh
sudo pacman -S rust pipewire-pulse libpulse
```

Debian / Ubuntu:

```sh
sudo apt install cargo libpulse-dev pipewire-pulse
```

Build:

```sh
cargo build --release
```

Binary:

```text
target/release/i3status-dumb
```

## Run

Inside Sway:

```sh
./target/release/i3status-dumb
```

## Use With Swaybar

```conf
bar {
    status_command /path/to/i3status-dumb
}
```

Or install:

```sh
sudo install -m755 target/release/i3status-dumb /usr/local/bin/i3status-dumb
```

Then:

```conf
bar {
    status_command i3status-dumb
}
```

## Notes

* Layout comes from Sway input metadata
* Mappings:

  * `English (US)` → `us`
  * `Russian` → `ru`
  * `Ukrainian` → `ua`
* Others fall back to the first 3 lowercase ASCII letters
* No PulseAudio → `??%`
* No Sway IPC → `??`

## Status

Intentionally opinionated. Intentionally limited.

That is the feature.
