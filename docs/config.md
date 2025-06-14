# Kumo

## Syntax

Kumo's configuration file uses the TOML format. The format's specification
can be found at _https://toml.io/en/v1.0.0_.

## Location

Kumo doesn't create the configuration file for you, but it looks for one at
<br> `${XDG_CONFIG_HOME:-$HOME/.config}/kumo/kumo.toml`.

## Fields

### font

This section documents the `[font]` table.

|Name|Description|Type|Default|
|-|-|-|-|
|family|Font family|text|`"sans"`|
|size|Font size|float|`16.0`|

### colors

This section documents the `[color]` table.

|Name|Description|Type|Default|
|-|-|-|-|
|fg|Primary foreground color|color|`"#ffffff"`|
|bg|Primary background color|color|`"#181818"`|
|hl|Primary accent color|color|`"#752a2a"`|
|secondary_fg|Secondary foreground color|color|`"#bfbfbf"`|
|secondary_bg|Secondary background color|color|`"#282828"`|
|error|Error foreground color|color|`"#ac4242"`|
|disabled|Disabled foreground color|color|`"#666666"`|

### input

This section documents the `[input]` table.

|Name|Description|Type|Default|
|-|-|-|-|
|max_tap_distance|Square of the maximum distance before touch input is considered a drag|float|`400.0`|
|max_multi_tap|Maximum interval between taps to be considered a double/trible-tap|integer (milliseconds)|`300`|
|long_press|Minimum time before a tap is considered a long-press|integer (milliseconds)|`300`|
|velocity_interval|Microseconds per velocity tick|float|`30000.0`|
|velocity_friction|Percentage of velocity retained each tick|float|`0.85`|
