use std::{io::Write, time::Duration};

mod midi;

use clap::{Args, Parser, Subcommand};

use capnp::message::Builder;
use capnp::serialize;
use midi::MidiColors;
use smart_leds::RGB8;

pub mod usb_messages_capnp {
    include!(concat!(env!("OUT_DIR"), "/usb_messages_capnp.rs"));
}

#[derive(Parser)]
struct Cli {
    /// Serial port for communicating with the badge
    ///
    /// This is the management interface with capnp, not the debug interface
    ///
    /// Defaults to /dev/ttyACM0
    #[arg(short, long)]
    serial_port: Option<String>,

    /// Set the badge to a solid color, the color should be written in hex format
    /// like "#ff0000" for red, etc.
    #[arg(short = 'c', long)]
    solid_color: Option<String>,

    /// Frame buffer to send to the badge.
    ///
    /// The frame buffer is a string with 9 "css" colors separated by spaces
    /// like "#ff0000 #00ff00 [...]"
    #[arg(short, long)]
    frame_buffer: Option<String>,

    /// Demo application to use the badge with the midi interface
    /// This does not do anything useful, it's just a demo to show
    /// how to use the midi interface
    ///
    /// The argument is the path to a midi device
    /// For example: /dev/midi3c
    #[arg(short, long)]
    midi_demo: Option<String>,

    #[command(subcommand)]
    subcommand: Option<Subcommands>,
}

#[derive(Subcommand)]
enum Subcommands {
    /// Use the badge to send an infrared NEC command
    SendNec(SendNec),
}

#[derive(Args, Debug)]
struct SendNec {
    /// NEC address
    #[arg(short, long)]
    address: u8,
    /// NEC command
    #[arg(short, long)]
    command: u8,
    /// Repeat
    #[arg(short, long)]
    repeat: bool,
}

fn hex_color_to_rgb(color: String) -> RGB8 {
    let color = color.trim_start_matches("#");
    let r = u8::from_str_radix(&color[0..2], 16).unwrap();
    let g = u8::from_str_radix(&color[2..4], 16).unwrap();
    let b = u8::from_str_radix(&color[4..6], 16).unwrap();
    RGB8 { r, g, b }
}

fn midi_demo(file: String) {
    let mut lp = MidiColors::new(&file).expect("Failed to open device");
    for x in 0..3 {
        for y in 0..3 {
            lp.led_ctrl_rgb(x, y, x * 127 / 3, y * 127 / 3, 0)
                .expect("Failed to set LED color");
        }
    }

    lp.led_ctrl_rgb(1, 1, 0, 0, 127)
        .expect("Failed to set LED color");
}

fn main() {
    let args = Cli::parse();

    // we don't need serial for the midi demo
    // let it f*ck off before everything else
    // ideally, this whole tool would support both backends,
    // for now, this is only here as a reference
    if let Some(file) = args.midi_demo {
        midi_demo(file);
        return;
    }

    let serial_port = args.serial_port.unwrap_or("/dev/ttyACM0".to_string());

    let mut port = serialport::new(serial_port, 115_200)
        .timeout(Duration::from_millis(10))
        .open()
        .expect("Failed to open port");

    #[allow(clippy::single_match)]
    match args.subcommand {
        Some(Subcommands::SendNec(send_nec)) => {
            let mut message = Builder::new_default();

            let badgebound = message.init_root::<usb_messages_capnp::badge_bound::Builder>();

            let mut nec = badgebound.init_send_nec_command();
            nec.set_address(send_nec.address);
            nec.set_command(send_nec.command);
            nec.set_repeat(send_nec.repeat);

            let data = serialize::write_message_to_words(&message);

            port.write_all(&data).expect("Failed to write to port");
        }
        None => {}
    }

    if let Some(fb) = args.frame_buffer {
        let split = fb
            .split(" ")
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        if split.len() != 9 {
            println!("Frame buffer must be 9 elements long");
            return;
        }

        let mut message = Builder::new_default();

        let badgebound = message.init_root::<usb_messages_capnp::badge_bound::Builder>();

        let mut set_fb = badgebound.init_set_frame_buffer();
        set_fb.reborrow().init_pixels(9);

        let mut pixels = set_fb.reborrow().get_pixels().unwrap();

        for i in 0..9 {
            let mut pixel = pixels.reborrow().get(i);
            let color = hex_color_to_rgb(split[i as usize].clone());
            pixel.set_r(color.r);
            pixel.set_g(color.g);
            pixel.set_b(color.b);
        }

        let data = serialize::write_message_to_words(&message);

        port.write_all(&data).expect("Failed to write to port");

        return;
    }

    if let Some(color) = args.solid_color {
        let mut message = Builder::new_default();

        let badgebound = message.init_root::<usb_messages_capnp::badge_bound::Builder>();

        let mut set_color = badgebound.init_set_solid_color();
        let color = hex_color_to_rgb(color);

        set_color.set_r(color.r);
        set_color.set_g(color.g);
        set_color.set_b(color.b);

        let data = serialize::write_message_to_words(&message);

        port.write_all(&data).expect("Failed to write to port");
    }
}
