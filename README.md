# Kumo - A Wayland Mobile Browser

<p>
  <img src="./logo.svg" width="10%" align="left">

  Kumo is a web browser with a UI focused on portrait-mode touchscreen mobile
  devices. It is optimized to run on low-end hardware with a limited battery life.

  <br clear="align"/>
</p>

## Screenshots

<p align="center">
  <img src="https://github.com/catacombing/kumo/assets/8886672/ccff7958-0745-4757-8450-bf1782c19c45" width="50%"/>
</p>

## Installation

The following dependencies are required for running Kumo:

| Dependency        | Version          | Details                                                             |
| ----------------- | ---------------- | ------------------------------------------------------------------- |
| wpewebkit         | libWPEWebKit-2.0 |                                                                     |
| gst-plugins-good  | 1.0              | Required for audio/video playback; specifically `autodetect` plugin |
| gst-libav         | 1.0              | Required for audio/video playback                                   |

After compiling, the binary can be found at `./target/release/kumo`:

```sh
cargo build --release
```
