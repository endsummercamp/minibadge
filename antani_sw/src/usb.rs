use defmt::{info, panic};
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, Instance, InterruptHandler};

use crate::AppSender;
use embassy_usb::class::midi::MidiClass;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

#[embassy_executor::task]
pub async fn usb_main(usb: USB, control: AppSender) {
    info!("Hello world!");

    // Create the driver, from the HAL.
    let driver = Driver::new(usb, Irqs);

    // Create embassy-usb Config
    let mut config = Config::new(0x0000, 0x0000);
    config.manufacturer = Some("ESC");
    config.product = Some("mini badge");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    // Create classes on the builder.
    let mut midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    // let state = STATE.get().lock().await.ref_mut();

    // The `MidiClass` can be split into `Sender` and `Receiver`, to be used in separate tasks.
    // let (sender, receiver) = class.split();

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    // Use the Midi class!
    let midi_fut = async {
        loop {
            midi_class.wait_connection().await;
            info!("Connected");
            let _ = midi_echo(&mut midi_class, control).await;
            info!("Disconnected");
        }
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(usb_fut, midi_fut).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn midi_echo<'d, T: Instance + 'd>(
    class: &mut MidiClass<'d, Driver<'d, T>>,
    control: AppSender,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;

        // read at chunk of 4 bytes
        for i in (0..n).step_by(4) {
            //let data = &buf[i..i+4];
            let [_, _, button, value] = buf[i..i + 4]
                .try_into()
                .expect("slice with incorrect length");

            info!("pixel: {}, value: {}", button, value);

            // button 0 = pixel 0 red
            // button 1 = pixel 0 green
            // button 2 = pixel 0 blue
            // button 3 = pixel 1 red
            // etc etc

            let width = 3;

            let pixel = button / 3;
            let x = pixel % width;
            let y = pixel / width;
            let channel = button % 3;

            if x >= width || y >= width {
                continue;
            }

            // the (0,0) should be the top left pixel
            // TODO: x and y are probably wrong / inconsistent
            let x = width - x - 1;

            // warning: midi values are 0-127, we need to double them to get 0-255
            control
                .send(crate::TaskCommand::MidiSetPixel(x, y, channel, value * 2))
                .await;
        }

        // let data = &buf[..n];
        // info!("data: {:x}", data);
        // class.write_packet(data).await?;
    }
}
