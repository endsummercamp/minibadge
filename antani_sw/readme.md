# End Summer Camp - Mini Badge - Firmware

This is the firmware for the End Summer Camp Mini Badge.

## Build and Flash

You need to have a working Rust toolchain installed. You can install it with your distribution's package manager, or follow the instructions on the [Rust website](https://www.rust-lang.org/tools/install).

When you have a working "cargo" command, you can build and flash the firmware with the following steps:

1. Clone this repository and navigate to the `antani_sw` directory
2. Disconnect the badge from USB
3. Connect the badge to USB while holding the BOOT button
4. Mount the RP2040's bootloader virtual USB drive
5. Run `cargo run --release`.

The badge should now reboot with the new firmware.

## USB

The badge exposes one MIDI device and two CDC devices over USB. The MIDI device is used to control the lights with MIDI messages, and the CDC devices are used for debugging and controlling the badge.


The first CDC device is to be used with the `minibadge-cli` tool, that communicates with the badge using a protocol based on Cap'n Proto. You can find the CLI tool in the `minibadge-cli` directory.

The second CDC device is used for debugging and logging. You can connect to it with a serial terminal at 115200 baud, for example

```sh
sudo picocom -b 115200 --imap lfcrlf /dev/ttyACM1
```

