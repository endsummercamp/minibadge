# End Summer Camp - Mini Badge - Command Line Tool

This is the command line tool for the End Summer Camp Mini Badge.

## Usage

If you don't have a Rust toolchain installed, follow the instructions in the firmware readme
(directory `/antani_sw`).

Additionally, the build process requires the `capnp` binary to be installed in
your system. Please be sure it is installed before running the CLI tool.

To run the CLI tool, just run `cargo run -- --help` in this directory.

```
> cargo run -q -- --help
Usage: minibage-cli [OPTIONS] [COMMAND]

Commands:
  send-nec  Use the badge to send an infrared NEC command
  help      Print this message or the help of the given subcommand(s)

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
          
          The argument is the path to a midi device For example: /dev/midi3c

  -h, --help
          Print help (see a summary with '-h')

```

### Infrared subcommand

```
> cargo run -q -- help send-nec
Use the badge to send an infrared NEC command

Usage: minibage-cli send-nec [OPTIONS] --address <ADDRESS> --command <COMMAND>

Options:
  -a, --address <ADDRESS>  NEC address
  -c, --command <COMMAND>  NEC command
  -r, --repeat             Repeat
  -h, --help               Print help
```

IR commands can be debugged / received with the badge itself, just open the debug CDC interface with a serial terminal.

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

```sh
cargo run -q -- -s /dev/ttyACM0  send-nec --address 7 --command 22
```

## More examples

Random solid color:

```sh
while true ; do red=$((1 + $RANDOM % 128)) ; green=$((1 + $RANDOM % 128)) ; blue=$((1 + $RANDOM % 128)) ; hred=$(printf "%#04x" $red) ; hgreen=$(printf "%#04x" $green) ; hblue=$(printf "%#04x" $blue) ; hred=${hred:2:2} ; hgreen=${hgreen:2:2} ; hblue=${hblue:2:2} ; cargo run -q -- --solid-color "#$hred$hgreen$hblue" ; sleep 0 ; done
```

Random colors:
```sh
frameBuffer="#" ; for i in {0..8} ; do for j in {0..2} ; do color=$((1 + $RANDOM % 255)) ; hcolor=$(printf "%#04x" $color) ; hcolor=${hcolor:2:2} ; frameBuffer+=$hcolor ; done ; if [[ $i -lt 8 ]] ; then frameBuffer+=" #" ; fi ; done ; cargo run -q -- --frame-buffer "$frameBuffer"
```

or

```sh
led=() ; frameBuffer="" ; for i in {1..9} ; do ledSingle="#" ; for j in {0..2} ; do color=$((1 + $RANDOM % 255)) ; hcolor=$(printf "%#04x" $color) ; hcolor=${hcolor:2:2} ; ledSingle+=$hcolor ; done ; led+=($ledSingle)  ; if [[ $i -lt 8 ]] ; then frameBuffer+=" #" ; fi ; done ; cargo run -q -- --frame-buffer "${led[1]} ${led[2]} ${led[3]} ${led[4]} ${led[5]} ${led[6]} ${led[7]} ${led[8]} ${led[9]}"
```

Random color, scrolling (not so noticeable):
```sh
pause=0.5 ; led=() ; frameBuffer="" ; for i in {1..9} ; do ledSingle="#" ; for j in {0..2} ; do color=$((1 + $RANDOM % 255)) ; hcolor=$(printf "%#04x" $color) ; hcolor=${hcolor:2:2} ; ledSingle+=$hcolor ; done ; led+=($ledSingle) ; if [[ $i -lt 8 ]] ; then frameBuffer+=" #" ; fi ; done ; cargo run -q -- --frame-buffer "${led[1]} ${led[2]} ${led[3]} ${led[4]} ${led[5]} ${led[6]} ${led[7]} ${led[8]} ${led[9]}" ; while true ; do old=1 ; new=2 ; for i in {0..7} ; do led[$old]=${led[$new]} ; old=$new ; new=$((new+1)) ; done ; led[9]="" ; ledSingle="#" ; for j in {0..2} ; do color=$((1 + $RANDOM % 255)) ; hcolor=$(printf "%#04x" $color) ; hcolor=${hcolor:2:2} ; ledSingle+=$hcolor ; done ; led[9]=($ledSingle) ; sleep $pause ; cargo run -q -- --frame-buffer "${led[1]} ${led[2]} ${led[3]} ${led[4]} ${led[5]} ${led[6]} ${led[7]} ${led[8]} ${led[9]}" ; done
```
