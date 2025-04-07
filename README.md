# Kumo - A Wayland Mobile Browser

<p>
  <img src="./logo.svg" width="10%" align="left">

  Kumo is a web browser with a UI focused on portrait-mode touchscreen mobile
  devices. It is optimized to run on low-end hardware with a limited battery life.

  <br clear="align"/>
</p>

## Features

Kumo is a UI written around Safari's WebKit browser engine, so they should be
comparable in behavior and performance.

The following noteworthy features are implemented:

 - Built-in adblocker
 - Whitelist-based cookie policy
 - Shell-like URI completion
 - Session recovery
 - Tab groups
 - History management

## Demo

https://github.com/user-attachments/assets/db0bee78-9db2-439b-beb4-1020ff889008

## Installation

The following dependencies are required for running Kumo:

| Dependency        | Version          | Details                                                                  |
| ----------------- | ---------------- | ------------------------------------------------------------------------ |
| wpewebkit         | libWPEWebKit-2.0 |                                                                          |
| gst-plugins-base  | 1.0              | (Optional) Required for media playback; specifically OpenGL plugin       |
| gst-plugins-good  | 1.0              | (Optional) Required for media playback; specifically `autodetect` plugin |
| gst-plugins-bad   | 1.0              | (Optional) Required for media playback; specifically `fdkaac` plugin     |
| gst-libav         | 1.0              | (Optional) Required for non-free media playback                          |

After compiling, the binary can be found at `./target/release/kumo`:

```sh
cargo build --release
```
