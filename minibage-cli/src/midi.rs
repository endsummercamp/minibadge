

use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
};

#[allow(dead_code)]
pub struct MidiEvent {
    pub key: u8,
    pub is_pressed: bool,
}

#[allow(dead_code)]
impl MidiEvent {
    pub fn new(key: u8, pressed: bool) -> Self {
        Self {
            key,
            is_pressed: pressed,
        }
    }

    pub fn key_pressed(&self, key: u8) -> bool {
        self.key == key && self.is_pressed
    }

    pub fn key_released(&self, key: u8) -> bool {
        self.key == key && !self.is_pressed
    }
}

pub struct MidiColors {
    fd: std::fs::File,
}

impl MidiColors {
    pub fn new(dev: &str) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(dev)?;
        Ok(Self { fd: file })
    }

    pub fn led_ctrl_raw(&mut self, button: u8, color: u8) -> io::Result<()> {
        let data = [0x90, button, color];
        self.fd.write_all(&data)?;
        self.fd.flush()?;
        Ok(())
    }

    pub fn led_ctrl_rgb(&mut self, x: u8, y: u8, red: u8, green: u8, blue: u8) -> io::Result<()> {
        // button 0 = pixel 0 red
        // button 1 = pixel 0 green
        // button 2 = pixel 0 blue
        // button 3 = pixel 1 red
        // etc etc

        if red > 127 || green > 127 || blue > 127 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Color values must be less than 128",
            ));
        }

        let button = (y + 3 * x) * 3;

        println!(
            "Setting button {} to rgb {},{},{}",
            button, red, green, blue
        );
        self.led_ctrl_raw(button, red)?;
        self.led_ctrl_raw(button + 1, green)?;
        self.led_ctrl_raw(button + 2, blue)?;

        Ok(())
    }

    #[allow(dead_code)]
    pub fn wait_event(&mut self) -> io::Result<MidiEvent> {
        let mut data = [0u8; 3];
        self.fd.read_exact(&mut data)?;
        Ok(MidiEvent::new(data[1], data[2] != 0))
    }
}
