# End Summer Camp - Mini Badge - Command Line Tool

This is the command line tool for the End Summer Camp Mini Badge.

## Usage

If you don't have a Rust toolchain installed, follow the instructions in the firmware readme.

To run the CLI tool, just run `cargo run -- --help` in this directory.

```
> cargo run -q -- --help
Usage: minibage-cli [OPTIONS]

Options:
  -s, --serial-port <SERIAL_PORT>
          Serial port for communicating with the badge
          
          This is the management interface with capnp, not the debug interface
          
          Defaults to /dev/ttyACM0

  -c, --solid-color <SOLID_COLOR>
          Set the badge to a solid color, the color should be written in hex format like "#ff0000" for red, etc

  -f, --frame-buffer <FRAME_BUFFER>
          Frame buffer to send to the badge.
          
          The frame buffer is a string with 9 "css" colors separated by spaces like "#ff0000 #00ff00 [...]"

  -m, --midi-demo <MIDI_DEMO>
          Demo application to use the badge with the midi interface This does not do anything useful, it's just a demo to show how to use the midi interface
          
          The argument is the path to a midi device For example: /dev/midi3

  -h, --help
          Print help (see a summary with '-h')
```

## Examples

```sh
cargo run -q -- -s /dev/ttyACM0 -c "#ff0000"
```

```sh
cargo run -q -- -s /dev/ttyACM0 -f "#ff0000 #00ff00 #0000ff #ff00ff #00ffff #ffff00 #ffffff #000000 #888888"
```

```sh
cargo run -q -- -m /dev/midi3
```