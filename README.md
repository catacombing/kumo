# Kumo - A Wayland Mobile Browser

Kumo is a web browser with a UI focused on portrait-mode touchscreen mobile
devices. It is optimized to run on low-end hardware with a limited battery life.

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
