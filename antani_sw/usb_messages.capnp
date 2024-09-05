@0x9966d27e7566db0b;

struct BadgeBound {
  union {
    null @0 :Void;
    setFrameBuffer @1 :SetFrameBuffer;
    setSolidColor @2 :RGB8;
    sendNecCommand @3 :NecCommand;
  }
}

struct SetFrameBuffer {
  pixels @0 :List(RGB8);
}

struct RGB8 {
  r @0 :UInt8;
  g @1 :UInt8;
  b @2 :UInt8;
}

struct NecCommand {
  address @0 :UInt8;
  command @1 :UInt8;
  repeat @2 :Bool;
}