fn main() {
    ::capnpc::CompilerCommand::new()
        .file("usb_messages.capnp")
        .run()
        .expect("compiling schema");
}
