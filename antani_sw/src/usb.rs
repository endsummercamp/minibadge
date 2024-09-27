use defmt::{panic, warn};
use embassy_futures::join::join;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Ipv4Cidr, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, Instance, InterruptHandler};
use embassy_time::{Duration, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as AcmState};
use embassy_usb::class::cdc_ncm::{ CdcNcmClass};
use embassy_usb::class::cdc_ncm::State as NcmState;
use embassy_usb::class::hid::{self, HidWriter};
use embedded_io_async::Write;
use heapless::{String, Vec};
use log::{error, info};
use rand::RngCore;
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, SerializedDescriptor};

use crate::{MegaPublisher, MegaSubscriber, TaskCommand};
use embassy_usb::class::cdc_ncm::embassy_net::{Device, Runner, State as NetState};
use embassy_usb::class::midi::MidiClass;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};


use defmt_rtt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

static STATE: StaticCell<AcmState> = StaticCell::new();
static LOGGER_STATE: StaticCell<AcmState> = StaticCell::new();
static HID_STATE: StaticCell<hid::State> = StaticCell::new();
static CONFIG_DESCRIPTOR: StaticCell<[u8; 512]> = StaticCell::new();
static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

const MTU: usize = 1514;

#[derive(Debug)]
struct Request {
    method: String<8>,
    path: String<32>,
}

struct MinHttpServer<'a> {
    stack: embassy_net::Stack<'a>,
}

impl<'a> MinHttpServer<'a> {
    pub fn new(stack: embassy_net::Stack<'a>) -> Self {
        Self { stack }
    }

    pub async fn parse_http_request(&mut self, request: &[u8]) -> Request {
        let mut method = String::new();
        let mut path = String::new();

        let mut iter = request.split(|&c| c == b' ');

        let method_bytes = iter.next().unwrap();
        let path_bytes = iter.next().unwrap();

        for &c in method_bytes {
            method.push(c as char).unwrap();
        }

        for &c in path_bytes {
            path.push(c as char).unwrap();
        }

        Request { method, path }
    }

    // callback does not return headers
    pub async fn run(&mut self, request_callback: impl Fn(Request) -> String<4>) {


        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        let mut buf = [0; 4096];


        loop {
            let mut socket = TcpSocket::new(self.stack, &mut rx_buffer, &mut tx_buffer);
            socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));
    
            info!("Listening on TCP:80...");
            
        if let Err(e) = socket.accept(8080).await {
            warn!("accept error: {:?}", e);
            return;
        }

        info!("Received connection from {:?}", socket.remote_endpoint());

            let n = match socket.read(&mut buf).await {
                
                Ok(0) => {
                    warn!("read EOF");
                    continue;
                }
                Ok(n) => n,
                Err(e) => {
                    warn!("read error: {:?}", e);
                    continue;
                }
            };

            let request = self.parse_http_request(&buf[..n]).await;

            info!("HTTP request: {:?}", request);

            let status = request_callback(request);

            socket.write_all("HTTP/1.1 ".as_bytes()).await.unwrap();
            socket.write_all(status.as_bytes()).await.unwrap();
            socket.write_all(" OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\nOK".as_bytes()).await.unwrap();
            socket.write_all(status.as_bytes()).await.unwrap();

            socket.flush().await.unwrap();

            socket.close();
        }
    }
}



async fn network_stack(stack: embassy_net::Stack<'_>) {

    // let mut rx_buffer = [0; 4096];
    // let mut tx_buffer = [0; 4096];
    // let mut buf = [0; 4096];

    let mut http_server = MinHttpServer::new(stack);

    http_server.run(|request| {
        info!("HTTP request: {} {}", request.method, request.path);

        let mut status = String::new();
        status.push_str("200").unwrap();
        status
    }).await;

    // loop {
    //     let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    //     socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    //     info!("Listening on TCP:1234...");
    //     if let Err(e) = socket.accept(1234).await {
    //         warn!("accept error: {:?}", e);
    //         continue;
    //     }

    //     info!("Received connection from {:?}", socket.remote_endpoint());

    //     loop {
    //         let n = match socket.read(&mut buf).await {
    //             Ok(0) => {
    //                 warn!("read EOF");
    //                 break;
    //             }
    //             Ok(n) => n,
    //             Err(e) => {
    //                 warn!("read error: {:?}", e);
    //                 break;
    //             }
    //         };

    //         info!("rxd {:?}", &buf[..n]);

    //         match socket.write_all(&buf[..n]).await {
    //             Ok(()) => {}
    //             Err(e) => {
    //                 warn!("write error: {:?}", e);
    //                 break;
    //             }
    //         };
    //     }
    // }
}

#[embassy_executor::task]
pub async fn usb_main(usb: USB, publisher: MegaPublisher, mut subscriber: MegaSubscriber) {
    // Create the driver, from the HAL.
    let driver = Driver::new(usb, Irqs);

    // Create embassy-usb Config
    let mut config = Config::new(0x0000, 0x0000);
    config.manufacturer = Some("ESC");
    config.product = Some("Mini Badge");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 512]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 256]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        &mut [], // no msos descriptors
        control_buf,
    );

    let mut midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    let acm_state = STATE.init(AcmState::new());
    let logger_state = LOGGER_STATE.init(AcmState::new());
    let hid_state = HID_STATE.init(hid::State::new());

    let config = embassy_usb::class::hid::Config {
        report_descriptor: KeyboardReport::desc(),
        request_handler: None,
        poll_ms: 60,
        max_packet_size: 64,
    };
    let mut hid_writer = HidWriter::<_, 8>::new(&mut builder, hid_state, config);

    let mut cdc_class = CdcAcmClass::new(&mut builder, acm_state, 64);
    let logger_class = CdcAcmClass::new(&mut builder, logger_state, 64);

    let log_fut = embassy_usb_logger::with_custom_style!(
        1024,
        log::LevelFilter::Info,
        logger_class,
        |record, writer| {
            use core::fmt::Write;
            let level = record.level().as_str();
            write!(writer, "[{level}] {}\r\n", record.args()).unwrap();
        }
    );

    // usb network adapter

    // Our MAC addr.
    let our_mac_addr = [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];
    // Host's MAC addr. This is the MAC the host "thinks" its USB-to-ethernet adapter has.
    let host_mac_addr = [0x42, 0x42, 0x42, 0x42, 0x42, 0x42];

    // Create classes on the builder.
    static NCM_STATE: StaticCell<NcmState> = StaticCell::new();
    let ncm_class = CdcNcmClass::new(
        &mut builder,
        NCM_STATE.init(NcmState::new()),
        host_mac_addr,
        64,
    );

    static NET_STATE: StaticCell<NetState<MTU, 4, 4>> = StaticCell::new();
    let (net_device_runner, device) = ncm_class
        .into_embassy_net_device::<MTU, 4, 4>(NET_STATE.init(NetState::new()), our_mac_addr);


    // let config = embassy_net::Config::dhcpv4(Default::default());
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
       address: Ipv4Cidr::new(Ipv4Address::new(10, 42, 0, 61), 24),
       dns_servers: Vec::new(),
       gateway: Some(Ipv4Address::new(10, 42, 0, 1)),
    });


    // Generate random seed
    let mut rng = embassy_rp::clocks::RoscRng;
    let seed = rng.next_u64();

    // Init network stack
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, mut net_stack_runner) = embassy_net::new(device, config, RESOURCES.init(StackResources::new()), seed);
    

    // Start network stack
    let network_fut = network_stack(stack);

    let mut usb = builder.build();

    let usb_fut = usb.run();

    let hid_fut = async {
        loop {
            if let TaskCommand::SendHidKeyboard(cmd) = subscriber.next_message_pure().await {
                let report = KeyboardReport {
                    keycodes: [cmd as u8, 0, 0, 0, 0, 0],
                    leds: 0,
                    modifier: 0,
                    reserved: 0,
                };
                // Send the report.
                match hid_writer.write_serialize(&report).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!("Failed to send report: {:?}", e);
                        publisher.publish(TaskCommand::Error).await;
                    }
                };
                Timer::after(Duration::from_millis(100)).await;

                let report = KeyboardReport {
                    keycodes: [0, 0, 0, 0, 0, 0],
                    leds: 0,
                    modifier: 0,
                    reserved: 0,
                };
                match hid_writer.write_serialize(&report).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!("Failed to send report: {:?}", e);
                        publisher.publish(TaskCommand::Error).await;
                    }
                };
            }
        }
    };

    let midi_fut = async {
        loop {
            midi_class.wait_connection().await;
            info!("Connected");
            let _ = midi_echo(&mut midi_class, &publisher).await;
            info!("Disconnected");
        }
    };

    let control_fut = async {
        loop {
            cdc_class.wait_connection().await;
            info!("Connected");
            let _ = usb_control(&mut cdc_class, &publisher).await;
            info!("Disconnected");
        }
    };

    let net_stack_future = async {
        loop {
            net_stack_runner.run().await;
        }
    };

    let net_device_future = async {
        loop {
            net_device_runner.run().await;
        }
    };

    join(
        usb_fut,
        join(control_fut, join(log_fut, join(hid_fut, join(midi_fut, join(network_fut, join(net_stack_future, net_device_future))))))
    )
    .await;
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
    publisher: &MegaPublisher,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;

        // read at chunk of 4 bytes
        for i in (0..n).step_by(4) {
            //let data = &buf[i..i+4];
            let buf: &[u8; 4] = match buf[i..i + 4].try_into() {
                Ok(buf) => buf,
                Err(_) => {
                    warn!("got bad midi data");
                    continue;
                }
            };

            let [_, _, button, value] = buf;

            info!("midi pixel: {}, value: {}", button, value);

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
            publisher
                .publish(crate::TaskCommand::MidiSetPixel(x, y, channel, value * 2))
                .await;
        }
    }
}

struct AlignedVec {
    x: Vec<u8, 256>,
    _alignment: [u64; 0],
}

impl AlignedVec {
    fn new() -> Self {
        Self {
            x: Vec::<u8, 256>::new(),
            _alignment: [0; 0],
        }
    }
}

async fn usb_control<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
    publisher: &MegaPublisher,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    let mut mega_deserialization_buf = AlignedVec::new();
    loop {
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        info!("usb cdc data: {:?}", data);

        // append to the mega deserialization buffer
        // we don't really care if it fails, we'll just clear it later
        mega_deserialization_buf.x.extend_from_slice(data).ok();

        let e = crate::capnp::deserialize_message(&mut mega_deserialization_buf.x.as_slice());

        match e {
            Ok(command) => {
                info!("Deserialized message");

                mega_deserialization_buf.x.clear();

                publisher.publish(command).await;
                publisher.publish(crate::TaskCommand::UsbActivity).await;
            }
            Err(e) => match e.kind {
                capnp::ErrorKind::MessageEndsPrematurely(_, _) => {
                    continue;
                }

                e => {
                    error!("Error deserializing message: {:?}", e);

                    publisher.publish(crate::TaskCommand::Error).await;

                    mega_deserialization_buf.x.clear();
                }
            },
        }
    }
}
